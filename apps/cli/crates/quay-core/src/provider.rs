//! PR/MR opener abstraction. Keeps `quay-core` independent of any specific
//! hosting provider while allowing `quay-cli` to inject real or fake openers.

use crate::error::{QuayError, Result};
use std::path::Path;
use std::process::Command;

/// Result of a successful PR/MR open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    /// User-visible URL (PR page, or hint to open one manually).
    pub url: String,
    /// True when the PR was created automatically; false when the caller still
    /// needs to open the PR/MR by hand (printed URL is a hint, not a link to a real PR).
    pub auto_created: bool,
}

/// Opens a PR/MR after a branch has been pushed.
pub trait PrOpener {
    /// Open (or prepare the URL for) a pull request on the hosting provider.
    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo>;
}

/// Uses `gh pr create` if the binary is installed; otherwise returns a [`PrInfo`]
/// pointing the user at the GitHub compare URL for manual PR creation.
pub struct GhCliOpener;

impl Default for GhCliOpener {
    fn default() -> Self {
        Self
    }
}

impl GhCliOpener {
    fn gh_available(&self) -> bool {
        Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

impl PrOpener for GhCliOpener {
    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        // Resolve the origin URL first — we need it for both the fallback hint and
        // to decide whether `gh pr create` is even applicable (GitHub remotes only).
        let origin_out = Command::new("git")
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
        if !origin_out.status.success() {
            let stderr = String::from_utf8_lossy(&origin_out.stderr).to_string();
            return Err(QuayError::ConfigValidation(format!(
                "git remote get-url origin failed: {}",
                stderr.trim()
            )));
        }
        let origin_url = String::from_utf8_lossy(&origin_out.stdout)
            .trim()
            .to_string();
        let is_github = origin_url.contains("github.com");

        if !self.gh_available() || !is_github {
            // Fallback: produce the GitHub compare URL the user can open in a browser,
            // or a placeholder when the remote is not a GitHub URL (e.g. local bare repo
            // used in tests).
            if is_github {
                // Convert https://github.com/owner/repo.git → compare URL.
                let compare =
                    origin_url.trim_end_matches(".git").to_string() + "/pull/new/" + branch;
                return Ok(PrInfo {
                    url: compare,
                    auto_created: false,
                });
            } else {
                return Ok(PrInfo {
                    url: format!("{}/pull/new/{}", origin_url.trim_end_matches('/'), branch),
                    auto_created: false,
                });
            }
        }

        let out = Command::new("gh")
            .arg("-R")
            .arg(repo)
            .arg("pr")
            .arg("create")
            .arg("--head")
            .arg(branch)
            .arg("--title")
            .arg(title)
            .arg("--body")
            .arg(body)
            .output()
            .map_err(|source| QuayError::Io {
                path: "gh pr create".into(),
                source,
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(QuayError::ConfigValidation(format!(
                "gh pr create failed: {}",
                stderr.trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok(PrInfo {
            url: stdout,
            auto_created: true,
        })
    }
}

/// Test-only opener that never shells out — produces a deterministic fake [`PrInfo`].
#[cfg(any(test, debug_assertions))]
pub struct FakeOpener;

#[cfg(any(test, debug_assertions))]
impl PrOpener for FakeOpener {
    fn open_pr(&self, _repo: &Path, branch: &str, _title: &str, _body: &str) -> Result<PrInfo> {
        Ok(PrInfo {
            url: format!("https://example.test/pull/new/{}", branch),
            auto_created: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_opener_returns_branch_url() {
        let o = FakeOpener;
        let info = o
            .open_pr(Path::new("."), "feature/x", "title", "body")
            .unwrap();
        assert!(info.url.contains("feature/x"));
        assert!(!info.auto_created);
    }
}
