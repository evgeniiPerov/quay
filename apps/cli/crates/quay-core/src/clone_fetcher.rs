//! Provider-agnostic registry/skill fetcher based on `git clone --depth 1`.
//!
//! Works with GitHub, GitLab, Bitbucket, Azure DevOps, and any self-hosted
//! git server.  The default branch is used automatically (HEAD → whatever the
//! server advertises), so there is no need to pass a branch name.
//!
//! Within a single `CloneFetcher` instance, repeated calls for the same URL
//! reuse the first clone (in-memory cache keyed by URL).  Across CLI
//! invocations the cache does not persist — tempdirs are dropped when the
//! fetcher is dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{QuayError, Result};
use crate::fetcher::{RegistryFetcher, SkillFileFetcher};
use crate::registry::Registry;

/// Provider-agnostic fetcher that shallow-clones a hub URL into a tempdir.
///
/// Cache entries live for the lifetime of the fetcher (one CLI invocation).
#[derive(Default)]
pub struct CloneFetcher {
    /// Maps normalised URL → path to cloned working tree.
    cache: HashMap<String, PathBuf>,
    /// Owns the tempdirs so they are not deleted before the fetcher is dropped.
    _tempdirs: Vec<tempfile::TempDir>,
}

impl CloneFetcher {
    /// Create a new, empty fetcher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Shallow-clone `url` into a fresh tempdir if not already cached.
    ///
    /// Returns the path to the working tree root.
    fn ensure_clone(&mut self, url: &str) -> Result<&Path> {
        // Avoid borrowing `self` for both the check and the insert.
        if self.cache.contains_key(url) {
            return Ok(self.cache.get(url).expect("key was just confirmed present"));
        }

        let tmp = tempfile::tempdir().map_err(|e| QuayError::Io {
            path: "<tempdir>".into(),
            source: e,
        })?;

        let output = Command::new("git")
            .args(["clone", "--depth", "1", "--quiet", url, "."])
            .current_dir(tmp.path())
            // Suppress SSH host-key banners and other git diagnostics from
            // leaking onto the user's TTY.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| QuayError::Io {
                path: format!("git clone {url}"),
                source: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(QuayError::InvalidConfig {
                path: url.into(),
                reason: format!(
                    "git clone failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
            });
        }

        let work_path = tmp.path().to_path_buf();
        self._tempdirs.push(tmp);
        self.cache.insert(url.to_string(), work_path);
        Ok(self.cache.get(url).expect("just inserted"))
    }

    /// Read and parse `registry.json` from the cloned working tree.
    pub fn fetch_registry(&mut self, url: &str) -> Result<Registry> {
        let root = self.ensure_clone(url)?.to_path_buf();
        let registry_path = root.join("registry.json");
        let bytes = std::fs::read(&registry_path).map_err(|e| QuayError::Io {
            path: registry_path.display().to_string(),
            source: e,
        })?;
        let text = String::from_utf8(bytes).map_err(|e| QuayError::InvalidRegistry {
            reason: format!("registry.json is not valid UTF-8: {e}"),
        })?;
        Registry::parse(&text)
    }

    /// Read raw bytes of a file at `repo_path` inside the clone
    /// (e.g. `"skills/foo/SKILL.md"`).
    pub fn fetch_path(&mut self, url: &str, repo_path: &str) -> Result<Vec<u8>> {
        let root = self.ensure_clone(url)?.to_path_buf();
        let full_path = root.join(repo_path);
        std::fs::read(&full_path).map_err(|e| QuayError::Io {
            path: full_path.display().to_string(),
            source: e,
        })
    }

    /// Number of cached clones (exposed for tests).
    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self._tempdirs.len()
    }
}

impl RegistryFetcher for CloneFetcher {
    fn fetch(&self, hub_url: &str) -> Result<Registry> {
        // The trait takes `&self` but cloning requires `&mut self`.
        // We create a short-lived mutable helper; no persistent cache is shared
        // across trait-object call sites (that is fine — RegistryFetcher is
        // used at most once per remote per command invocation).
        let mut tmp_fetcher = CloneFetcher::new();
        tmp_fetcher.fetch_registry(hub_url)
    }
}

impl SkillFileFetcher for CloneFetcher {
    fn fetch_file(&self, hub_url: &str, path: &str) -> Result<Vec<u8>> {
        let mut tmp_fetcher = CloneFetcher::new();
        tmp_fetcher.fetch_path(hub_url, path)
    }

    /// `git clone --depth 1` always checks out HEAD; we cannot cheaply switch
    /// to an arbitrary ref in the same shallow clone without a full fetch.
    /// For pinned-SHA reads we clone again into a separate tempdir.
    fn fetch_file_at(&self, hub_url: &str, path: &str, git_ref: &str) -> Result<Vec<u8>> {
        let tmp = tempfile::tempdir().map_err(|e| QuayError::Io {
            path: "<tempdir>".into(),
            source: e,
        })?;

        // Clone with a specific branch/tag.  Commit SHAs require --no-single-branch
        // so that the shallow pack includes the target object.
        let clone_out = Command::new("git")
            .args([
                "clone", "--depth", "1", "--branch", git_ref, "--quiet", hub_url, ".",
            ])
            .current_dir(tmp.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| QuayError::Io {
                path: format!("git clone {hub_url}"),
                source: e,
            })?;

        if !clone_out.status.success() {
            let stderr = String::from_utf8_lossy(&clone_out.stderr);
            return Err(QuayError::InvalidConfig {
                path: hub_url.into(),
                reason: format!(
                    "git clone (ref {git_ref}) failed (exit {}): {}",
                    clone_out.status.code().unwrap_or(-1),
                    stderr.trim()
                ),
            });
        }

        let full_path = tmp.path().join(path);
        std::fs::read(&full_path).map_err(|e| QuayError::Io {
            path: full_path.display().to_string(),
            source: e,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Helper: create a bare git repo with a working tree that has a commit.
    /// Returns `(work_dir, bare_dir)` — both must stay alive for the test.
    fn make_bare_repo_with_files(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir) {
        let work_dir = tempfile::tempdir().unwrap();
        let bare_dir = tempfile::tempdir().unwrap();

        // Init bare repo.
        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(bare_dir.path())
            .output()
            .expect("git init --bare");

        // Init working copy.
        Command::new("git")
            .args(["init"])
            .current_dir(work_dir.path())
            .output()
            .expect("git init");

        for (k, v) in [("user.email", "test@quay"), ("user.name", "quay-test")] {
            Command::new("git")
                .args(["config", k, v])
                .current_dir(work_dir.path())
                .status()
                .expect("git config");
        }

        // Write files.
        for (rel_path, content) in files {
            let full = work_dir.path().join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, content).unwrap();
        }

        // Stage and commit.
        Command::new("git")
            .args(["add", "."])
            .current_dir(work_dir.path())
            .status()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(work_dir.path())
            .status()
            .expect("git commit");

        // Add remote and push.
        let bare_url = bare_dir.path().to_str().unwrap().to_string();
        Command::new("git")
            .args(["remote", "add", "origin", &bare_url])
            .current_dir(work_dir.path())
            .status()
            .expect("git remote add");

        let branch_out = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(work_dir.path())
            .output()
            .expect("git rev-parse HEAD");
        let branch = String::from_utf8_lossy(&branch_out.stdout)
            .trim()
            .to_string();

        Command::new("git")
            .args(["push", "origin", &format!("{branch}:{branch}")])
            .current_dir(work_dir.path())
            .status()
            .expect("git push");

        (work_dir, bare_dir)
    }

    const REGISTRY_JSON: &str = r#"{
        "hub": "test-hub",
        "generated_at": "2026-05-10T00:00:00Z",
        "schema_version": 1,
        "skills": {
            "foo": {
                "version": "0.1.0",
                "description": "Foo skill",
                "tags": [],
                "path": "skills/foo",
                "sha": "deadbeef",
                "files": ["SKILL.md"]
            }
        }
    }"#;

    const SKILL_MD: &str = "---\nname: foo\ndescription: Foo skill\nversion: 0.1.0\n---\nbody\n";

    #[test]
    fn reads_registry_from_bare_repo() {
        let (_work, bare) = make_bare_repo_with_files(&[("registry.json", REGISTRY_JSON)]);
        let url = bare.path().to_str().unwrap();
        let mut fetcher = CloneFetcher::new();
        let reg = fetcher.fetch_registry(url).expect("fetch_registry");
        assert!(reg.entry("foo").is_some());
    }

    #[test]
    fn reads_skill_md() {
        let (_work, bare) = make_bare_repo_with_files(&[
            ("registry.json", REGISTRY_JSON),
            ("skills/foo/SKILL.md", SKILL_MD),
        ]);
        let url = bare.path().to_str().unwrap();
        let mut fetcher = CloneFetcher::new();
        let bytes = fetcher
            .fetch_path(url, "skills/foo/SKILL.md")
            .expect("fetch_path");
        assert_eq!(String::from_utf8(bytes).unwrap(), SKILL_MD);
    }

    #[test]
    fn caches_within_same_instance() {
        let (_work, bare) = make_bare_repo_with_files(&[("registry.json", REGISTRY_JSON)]);
        let url = bare.path().to_str().unwrap();
        let mut fetcher = CloneFetcher::new();

        fetcher.fetch_registry(url).expect("first fetch");
        fetcher.fetch_registry(url).expect("second fetch");

        // Only one tempdir should exist — second call reused the cache.
        assert_eq!(fetcher.cache_len(), 1, "expected cache_len == 1");
    }

    #[test]
    fn handles_default_branch_named_dev() {
        let work_dir = tempfile::tempdir().unwrap();
        let bare_dir = tempfile::tempdir().unwrap();

        Command::new("git")
            .args(["init", "--bare"])
            .current_dir(bare_dir.path())
            .output()
            .expect("git init --bare");

        Command::new("git")
            .args(["init", "-b", "dev"])
            .current_dir(work_dir.path())
            .output()
            .expect("git init -b dev");

        for (k, v) in [("user.email", "test@quay"), ("user.name", "quay-test")] {
            Command::new("git")
                .args(["config", k, v])
                .current_dir(work_dir.path())
                .status()
                .expect("git config");
        }

        std::fs::write(work_dir.path().join("registry.json"), REGISTRY_JSON).unwrap();

        Command::new("git")
            .args(["add", "."])
            .current_dir(work_dir.path())
            .status()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(work_dir.path())
            .status()
            .expect("git commit");

        let bare_url = bare_dir.path().to_str().unwrap().to_string();
        Command::new("git")
            .args(["remote", "add", "origin", &bare_url])
            .current_dir(work_dir.path())
            .status()
            .expect("git remote add");

        Command::new("git")
            .args(["push", "origin", "dev:dev"])
            .current_dir(work_dir.path())
            .status()
            .expect("git push");

        // Set HEAD to dev on the bare repo.
        Command::new("git")
            .args(["symbolic-ref", "HEAD", "refs/heads/dev"])
            .current_dir(bare_dir.path())
            .status()
            .expect("git symbolic-ref HEAD");

        let mut fetcher = CloneFetcher::new();
        let reg = fetcher
            .fetch_registry(&bare_url)
            .expect("fetch with dev branch");
        assert!(reg.entry("foo").is_some(), "skill 'foo' not found");
    }
}
