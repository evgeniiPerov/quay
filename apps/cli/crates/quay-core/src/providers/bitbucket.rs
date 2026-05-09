//! Bitbucket Cloud provider.
//!
//! Bitbucket has no first-party CLI in Plan 7a, so `open_pr` always falls
//! back to the compare URL (`auto_created: false`).

use crate::error::Result;
use crate::provider::{ConnectionStatus, PrInfo, Provider, ProviderKind, RepoCoords};
use crate::providers::shared::{origin_url, parse_two_segment_url};
use std::path::Path;

/// Provider implementation for Bitbucket Cloud.
pub struct BitbucketProvider;

impl Provider for BitbucketProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Bitbucket
    }

    fn parse_url(&self, url: &str) -> Result<RepoCoords> {
        parse_two_segment_url(url, "bitbucket")
    }

    fn open_pr(&self, repo: &Path, branch: &str, _title: &str, _body: &str) -> Result<PrInfo> {
        // No first-party CLI in 7a — always fall back to compare URL.
        let coords = self.parse_url(&origin_url(repo)?)?;
        Ok(PrInfo {
            url: self.compare_url(&coords, branch),
            auto_created: false,
        })
    }

    fn test_connection(&self, url: &str) -> Result<ConnectionStatus> {
        crate::providers::shared::test_connection_via_git(url)
    }

    fn compare_url(&self, c: &RepoCoords, branch: &str) -> String {
        format!(
            "https://{}/{}/{}/pull-requests/new?source={}&t=1",
            c.host, c.owner, c.repo, branch
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// Create a local bare repo with a `registry.json` committed at HEAD.
    ///
    /// Returns the path to the bare repo directory.
    fn init_bare_repo(dir: &std::path::Path) -> PathBuf {
        let bare = dir.join("repo.git");
        Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .status()
            .unwrap();
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
        std::fs::write(work.join("registry.json"), "{\"skills\":[]}").unwrap();
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
    fn parses_bitbucket_org() {
        let c = BitbucketProvider
            .parse_url("https://bitbucket.org/org-a/skills.git")
            .unwrap();
        assert_eq!(c.owner, "org-a");
        assert_eq!(c.repo, "skills");
    }

    #[test]
    fn open_pr_always_falls_back() {
        // Set up a local non-bare repo with a Bitbucket-shaped origin URL.
        // git remote get-url origin returns whatever was configured, so we can
        // set a fake bitbucket.org URL even though the actual transport is file://.
        let dir = tempfile::TempDir::new().unwrap();

        // Create a bare repo that we can push to (needs to be reachable by git).
        let bare = init_bare_repo(dir.path());
        let bare_url = format!("file://{}", bare.display());

        // Clone it to create a working repo.
        let work = dir.path().join("client");
        Command::new("git")
            .args(["clone", &bare_url])
            .arg(&work)
            .status()
            .unwrap();

        // Override the origin URL to a fake Bitbucket URL.
        // git remote get-url origin will return this fake URL, which the provider
        // uses to build the compare URL.
        Command::new("git")
            .args(["-C"])
            .arg(&work)
            .args([
                "remote",
                "set-url",
                "origin",
                "https://bitbucket.org/myorg/myrepo.git",
            ])
            .status()
            .unwrap();

        let info = BitbucketProvider
            .open_pr(&work, "feat/my-branch", "My PR", "body")
            .unwrap();

        assert!(!info.auto_created);
        assert!(
            info.url.contains("/pull-requests/new"),
            "expected pull-requests/new in URL, got: {}",
            info.url
        );
    }

    #[test]
    fn compare_url_uses_pull_requests() {
        let c = RepoCoords {
            host: "bitbucket.org".into(),
            owner: "o".into(),
            repo: "r".into(),
            url: "".into(),
        };
        assert!(BitbucketProvider
            .compare_url(&c, "feat/x")
            .contains("/pull-requests/new?source=feat/x"));
    }
}
