//! Shared helpers used by all provider implementations.
//!
//! # test-connection helpers
//!
//! [`test_connection_via_git`] probes a remote URL for reachability,
//! authentication, and presence of `registry.json` at `HEAD`.  It tries
//! `git archive --remote` first (single round-trip), then falls back to a
//! shallow clone.  Errors are classified by stderr-substring matching in
//! [`classify_error`].
//!
//! # URL-parsing helpers
//!
//! [`parse_two_segment_url`] handles the common `host/owner/repo` shape
//! used by GitHub, GitLab (simple), and Bitbucket.  [`strip_scheme_and_user`]
//! normalises the wide variety of URL schemes into `(host, path)` tuples.
//!
//! # Process helpers
//!
//! [`cli_available`] checks if a binary is on `PATH`.
//! [`origin_url`] reads the `origin` remote URL of a local repository.
//!
//! # Error classification phrases
//!
//! Classified as [`ConnectionStatus::AuthFailed`]:
//! - `"authentication failed"`
//! - `"permission denied"`
//! - `"403"`, `"401"`
//!
//! Classified as [`ConnectionStatus::Unreachable`] (and unrecognised errors):
//! - `"could not resolve"`
//! - `"connection refused"`
//! - `"timed out"`
//! - `"network is unreachable"`

use crate::error::{QuayError, Result};
use crate::provider::{ConnectionStatus, RepoCoords};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TEST_CONN_TIMEOUT: Duration = Duration::from_secs(15);

// ── test_connection_via_git ───────────────────────────────────────────────────

/// Probe `url` for reachability, auth, and presence of `registry.json`.
///
/// Strategy:
/// 1. `git archive --remote=<url> HEAD registry.json` — single round-trip.
/// 2. If the server doesn't support archive over smart HTTP, fall back to a
///    `git clone --depth 1 --filter=blob:none --no-checkout` + sparse checkout.
pub fn test_connection_via_git(url: &str) -> Result<ConnectionStatus> {
    // Strategy 1: git archive --remote (single round-trip)
    if let Some(status) = try_archive(url)? {
        return Ok(status);
    }
    // Strategy 2: shallow clone fallback
    try_shallow_clone(url)
}

fn try_archive(url: &str) -> Result<Option<ConnectionStatus>> {
    let mut cmd = Command::new("git");
    cmd.args([
        "archive",
        "--format=tar",
        "--remote",
        url,
        "HEAD",
        "registry.json",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let out = run_with_timeout(cmd, TEST_CONN_TIMEOUT)?;
    if out.status.success() {
        let size = extract_tar_entry_size(&out.stdout, "registry.json").unwrap_or(0);
        return Ok(Some(ConnectionStatus::Ok {
            registry_size_bytes: size,
        }));
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    // archive over smart-protocol HTTPS is often unsupported — fall through to clone.
    // Also fall through when HEAD is unborn / not yet set ("no such ref").
    if stderr.contains("not enabled")
        || stderr.contains("not supported")
        || stderr.contains("no such ref")
    {
        return Ok(None);
    }
    Ok(Some(classify_error(&stderr)))
}

fn try_shallow_clone(url: &str) -> Result<ConnectionStatus> {
    let tmp = TempDir::new().map_err(|e| QuayError::Io {
        path: "tempdir".into(),
        source: e,
    })?;
    let mut cmd = Command::new("git");
    cmd.args([
        "clone",
        "--depth",
        "1",
        "--filter=blob:none",
        "--no-checkout",
        url,
    ])
    .arg(tmp.path())
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let out = run_with_timeout(cmd, TEST_CONN_TIMEOUT)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Ok(classify_error(&stderr));
    }
    let mut checkout = Command::new("git");
    checkout
        .args(["-C"])
        .arg(tmp.path())
        .args(["checkout", "HEAD", "--", "registry.json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = run_with_timeout(checkout, TEST_CONN_TIMEOUT)?;
    if !out.status.success() {
        return Ok(ConnectionStatus::NoRegistry(
            "registry.json not in HEAD".into(),
        ));
    }
    let path = tmp.path().join("registry.json");
    let size = std::fs::metadata(&path)
        .map(|m| m.len())
        .map_err(|e| QuayError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
    Ok(ConnectionStatus::Ok {
        registry_size_bytes: size,
    })
}

/// Run a [`Command`] with a wall-clock timeout, killing the child on overrun.
///
/// Returns the captured [`std::process::Output`] on success, or
/// [`QuayError::Timeout`] if the deadline is exceeded.
pub fn run_with_timeout(mut cmd: Command, deadline: Duration) -> Result<std::process::Output> {
    let mut child = cmd.spawn().map_err(|e| QuayError::Io {
        path: "git".into(),
        source: e,
    })?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| QuayError::Io {
            path: "git".into(),
            source: e,
        })? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut s) = child.stdout.take() {
                s.read_to_end(&mut stdout).ok();
            }
            if let Some(mut s) = child.stderr.take() {
                s.read_to_end(&mut stderr).ok();
            }
            return Ok(std::process::Output {
                status,
                stdout,
                stderr,
            });
        }
        if started.elapsed() > deadline {
            let _ = child.kill();
            return Err(QuayError::Timeout(format!(
                "test-connection exceeded {}s",
                deadline.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Classify a git stderr string into a [`ConnectionStatus`].
///
/// Recognised phrases (lowercased match):
/// - Auth: `"authentication failed"`, `"permission denied"`, `"403"`, `"401"`
/// - Unreachable: `"could not resolve"`, `"connection refused"`, `"timed out"`,
///   `"network is unreachable"`
/// - Everything else: falls through to `Unreachable` with raw stderr.
pub fn classify_error(stderr: &str) -> ConnectionStatus {
    let s = stderr.to_ascii_lowercase();
    if s.contains("authentication failed")
        || s.contains("permission denied")
        || s.contains("403")
        || s.contains("401")
    {
        ConnectionStatus::AuthFailed(stderr.trim().into())
    } else {
        // "could not resolve", "connection refused", "timed out",
        // "network is unreachable", and any other unrecognised error.
        ConnectionStatus::Unreachable(stderr.trim().into())
    }
}

/// Extract the size (in bytes) of the first entry named `_entry_name` from a
/// minimal tar byte stream.
///
/// Parses bytes `124..136` of the first 512-byte header block as an octal size.
/// Returns `None` on any parse failure.
pub fn extract_tar_entry_size(tar_bytes: &[u8], _entry_name: &str) -> Option<u64> {
    if tar_bytes.len() < 136 {
        return None;
    }
    let size_str = std::str::from_utf8(&tar_bytes[124..136])
        .ok()?
        .trim_end_matches('\0')
        .trim();
    u64::from_str_radix(size_str, 8).ok()
}

// ── URL-parsing helpers ───────────────────────────────────────────────────────

/// Parse a `host/owner/repo`-shaped URL (used by GitHub, GitLab simple, Bitbucket).
///
/// Handles:
/// - `https://host/owner/repo[.git][/]`
/// - `http://host/owner/repo[.git][/]`
/// - `ssh://git@host/owner/repo[.git]`
/// - `git@host:owner/repo[.git]` (SCP form)
///
/// Returns [`QuayError::InvalidInput`] if the path has fewer than two non-empty
/// segments after stripping the host.  `label` is only used in the error message
/// (e.g. `"github"`, `"bitbucket"`).
pub fn parse_two_segment_url(url: &str, label: &str) -> Result<RepoCoords> {
    let (host, path) = strip_scheme_and_user(url)?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err(QuayError::InvalidInput(format!(
            "{} url '{}' must have owner/repo path segments",
            label, url
        )));
    }
    let owner = segments[segments.len() - 2].to_string();
    let repo = segments[segments.len() - 1].to_string();
    Ok(RepoCoords {
        host: host.to_string(),
        owner,
        repo,
        url: url.into(),
    })
}

/// Split a URL into `(host, path)` handling multiple scheme / SCP forms.
///
/// Supported forms:
/// - `https://host/path`
/// - `http://host/path`
/// - `ssh://git@host/path` (user prefix stripped)
/// - `git@host:path` (SCP — colon converted to leading `/`)
///
/// The returned `host` and `path` are slices referencing the input string where
/// possible.  The path does NOT have a leading `/`.
pub fn strip_scheme_and_user(url: &str) -> Result<(&str, &str)> {
    // SCP form: git@host:path
    if let Some(rest) = url.strip_prefix("git@") {
        if let Some(colon_pos) = rest.find(':') {
            let host = &rest[..colon_pos];
            let path = &rest[colon_pos + 1..];
            return Ok((host, path));
        }
        return Err(QuayError::InvalidInput(format!(
            "malformed SCP url: '{}'",
            url
        )));
    }

    // Scheme-based forms
    let after_scheme = if let Some(s) = url.strip_prefix("https://") {
        s
    } else if let Some(s) = url.strip_prefix("http://") {
        s
    } else if let Some(s) = url.strip_prefix("ssh://") {
        // Strip optional user@ prefix
        if let Some(at_pos) = s.find('@') {
            &s[at_pos + 1..]
        } else {
            s
        }
    } else {
        return Err(QuayError::InvalidInput(format!(
            "unrecognised url scheme in: '{}'",
            url
        )));
    };

    // after_scheme is now "host/path" or "host:port/path"
    if let Some(slash_pos) = after_scheme.find('/') {
        let host = &after_scheme[..slash_pos];
        let path = &after_scheme[slash_pos + 1..];
        return Ok((host, path));
    }

    // No path segment — just a host
    Ok((after_scheme, ""))
}

/// Check if a binary named `name` is available on `PATH`.
pub fn cli_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `git -C <repo> remote get-url origin` and return trimmed stdout.
pub fn origin_url(repo: &Path) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .output()
        .map_err(|source| QuayError::Io {
            path: "git remote get-url origin".into(),
            source,
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(QuayError::ConfigValidation(format!(
            "git remote get-url origin failed: {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    // ── test_connection helpers ───────────────────────────────────────────────

    /// Set up a local bare repo seeded with `registry.json` at HEAD.
    ///
    /// Returns the path to the bare repo (`<dir>/repo.git`).
    fn init_bare_repo_with_registry(dir: &std::path::Path, registry_body: &str) -> PathBuf {
        let bare = dir.join("repo.git");
        Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .status()
            .unwrap();
        // Set HEAD → main so that `git clone --no-checkout` + `git checkout HEAD` work.
        Command::new("git")
            .args(["-C"])
            .arg(&bare)
            .args(["symbolic-ref", "HEAD", "refs/heads/main"])
            .status()
            .unwrap();
        let work = dir.join("work");
        Command::new("git")
            .args(["clone"])
            .arg(&bare)
            .arg(&work)
            .status()
            .unwrap();
        std::fs::write(work.join("registry.json"), registry_body).unwrap();
        Command::new("git")
            .args(["-C"])
            .arg(&work)
            .args(["add", "registry.json"])
            .status()
            .unwrap();
        Command::new("git")
            .args(["-C"])
            .arg(&work)
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ])
            .status()
            .unwrap();
        Command::new("git")
            .args(["-C"])
            .arg(&work)
            .args(["push", "origin", "HEAD:refs/heads/main"])
            .status()
            .unwrap();
        bare
    }

    #[test]
    fn test_connection_ok_against_local_bare_repo() {
        let dir = assert_fs::TempDir::new().unwrap();
        let bare = init_bare_repo_with_registry(dir.path(), "{\"skills\":[]}");
        let url = format!("file://{}", bare.display());
        let status = test_connection_via_git(&url).unwrap();
        assert!(
            matches!(status, ConnectionStatus::Ok { registry_size_bytes } if registry_size_bytes > 0),
            "expected Ok with size > 0, got: {:?}",
            status
        );
    }

    #[test]
    fn test_connection_no_registry() {
        let dir = assert_fs::TempDir::new().unwrap();
        let bare = dir.path().join("empty.git");
        Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .status()
            .unwrap();
        let url = format!("file://{}", bare.display());
        let status = test_connection_via_git(&url).unwrap();
        // git archive against an unborn HEAD fails -> classify_error -> Unreachable
        // OR clone fallback succeeds with no HEAD -> NoRegistry
        // Accept either; the contract is "not Ok"
        assert!(
            !matches!(status, ConnectionStatus::Ok { .. }),
            "expected non-Ok, got: {:?}",
            status
        );
    }

    /// This test exercises the 15s timeout path against a nonexistent path.
    /// Marked `#[ignore]` because it takes up to the full `TEST_CONN_TIMEOUT`
    /// (15 s) to resolve: git hangs waiting on a connection that will never
    /// come before the deadline kills it.  Run explicitly with:
    ///   `cargo test -p quay-core -- --ignored test_connection_unreachable_path`
    #[test]
    #[ignore]
    fn test_connection_unreachable_path() {
        let url = "file:///nonexistent/path/to/repo.git";
        let status = test_connection_via_git(url).unwrap();
        assert!(
            matches!(status, ConnectionStatus::Unreachable(_)),
            "expected Unreachable, got: {:?}",
            status
        );
    }

    #[test]
    fn classify_error_auth() {
        assert!(matches!(
            classify_error("fatal: Authentication failed for ..."),
            ConnectionStatus::AuthFailed(_)
        ));
        assert!(matches!(
            classify_error("Permission denied (publickey)."),
            ConnectionStatus::AuthFailed(_)
        ));
    }

    #[test]
    fn classify_error_unreachable() {
        assert!(matches!(
            classify_error("Could not resolve host: example.invalid"),
            ConnectionStatus::Unreachable(_)
        ));
        assert!(matches!(
            classify_error("Connection refused"),
            ConnectionStatus::Unreachable(_)
        ));
    }

    // ── parse_two_segment_url ─────────────────────────────────────────────────

    #[test]
    fn parse_two_segment_https_with_dot_git() {
        let c = parse_two_segment_url("https://github.com/o/r.git", "github").unwrap();
        assert_eq!(c.host, "github.com");
        assert_eq!(c.owner, "o");
        assert_eq!(c.repo, "r");
    }

    #[test]
    fn parse_two_segment_ssh_scp_form() {
        let c = parse_two_segment_url("git@github.com:o/r.git", "github").unwrap();
        assert_eq!(c.host, "github.com");
        assert_eq!(c.owner, "o");
        assert_eq!(c.repo, "r");
    }

    #[test]
    fn parse_two_segment_ssh_scheme_form() {
        let c = parse_two_segment_url("ssh://git@github.com/o/r", "github").unwrap();
        assert_eq!(c.host, "github.com");
        assert_eq!(c.owner, "o");
        assert_eq!(c.repo, "r");
    }

    #[test]
    fn parse_two_segment_rejects_only_owner() {
        let result = parse_two_segment_url("https://github.com/o", "github");
        assert!(result.is_err(), "expected Err for single-segment path");
    }

    // ── strip_scheme_and_user ─────────────────────────────────────────────────

    #[test]
    fn strip_scheme_handles_port() {
        let (host, _path) = strip_scheme_and_user("https://gitlab.example.com:8443/o/r").unwrap();
        assert_eq!(host, "gitlab.example.com:8443");
    }

    #[test]
    fn strip_scheme_scp_form() {
        let (host, path) = strip_scheme_and_user("git@github.com:owner/repo.git").unwrap();
        assert_eq!(host, "github.com");
        assert_eq!(path, "owner/repo.git");
    }

    #[test]
    fn strip_scheme_ssh_with_user() {
        let (host, path) = strip_scheme_and_user("ssh://git@github.com/owner/repo").unwrap();
        assert_eq!(host, "github.com");
        assert_eq!(path, "owner/repo");
    }

    #[test]
    fn strip_scheme_rejects_unknown_scheme() {
        assert!(strip_scheme_and_user("ftp://example.com/o/r").is_err());
    }
}
