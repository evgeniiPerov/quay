//! GitHub and GitHub Enterprise provider implementation.
//!
//! Handles `github.com` (public) and GitHub Enterprise (both Cloud subdomain
//! `*.ghe.com` and on-prem custom domains).  The `is_enterprise` flag controls
//! the [`ProviderKind`] returned by [`GitHubProvider::kind`]; URL parsing and
//! PR creation logic are identical for both — `gh` CLI transparently handles
//! GHE Cloud, and respects the `GH_HOST` env-var for on-prem.

use crate::error::{QuayError, Result};
use crate::provider::{ConnectionStatus, PrInfo, Provider, ProviderKind, RepoCoords};
use crate::providers::shared::{cli_available, origin_url, parse_two_segment_url};
use std::path::Path;
use std::process::Command;

// ── GitHubProvider ────────────────────────────────────────────────────────────

/// Provider for GitHub.com and GitHub Enterprise (Cloud + on-prem).
///
/// Set `is_enterprise = true` when the remote is a GHE instance (either the
/// `*.ghe.com` Cloud subdomain or an on-prem custom domain).  This only
/// affects the value returned by [`Provider::kind`]; all other behaviour is
/// identical.
pub struct GitHubProvider {
    is_enterprise: bool,
}

impl GitHubProvider {
    /// Create a new provider instance.
    ///
    /// Pass `is_enterprise = false` for `github.com`, `true` for any GHE
    /// instance.
    pub fn new(is_enterprise: bool) -> Self {
        Self { is_enterprise }
    }
}

impl Provider for GitHubProvider {
    fn kind(&self) -> ProviderKind {
        if self.is_enterprise {
            ProviderKind::GitHubEnterprise
        } else {
            ProviderKind::GitHub
        }
    }

    fn parse_url(&self, url: &str) -> Result<RepoCoords> {
        // parse_two_segment_url already sets url on the returned RepoCoords.
        parse_two_segment_url(url, "github")
    }

    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        // Resolve the origin URL first — needed for both the gh invocation and the
        // compare-URL fallback.
        let remote_url = origin_url(repo)?;

        if !cli_available("gh") {
            let coords = self.parse_url(&remote_url)?;
            return Ok(PrInfo {
                url: self.compare_url(&coords, branch),
                auto_created: false,
            });
        }

        let out = Command::new("gh")
            .args([
                "-R",
                &remote_url,
                "pr",
                "create",
                "--head",
                branch,
                "--title",
                title,
                "--body",
                body,
            ])
            .output()
            .map_err(|e| QuayError::Io {
                path: "gh".into(),
                source: e,
            })?;

        if out.status.success() {
            return Ok(PrInfo {
                url: String::from_utf8_lossy(&out.stdout).trim().into(),
                auto_created: true,
            });
        }

        // gh failed (e.g. not authenticated, non-GitHub URL, or unsupported host).
        // Fall back to the browser compare URL so the user can open a PR manually.
        // If the URL doesn't parse as a GitHub URL (e.g. a local file path used in
        // tests), construct a best-effort URL from the raw remote URL.
        let fallback_url = match self.parse_url(&remote_url) {
            Ok(coords) => self.compare_url(&coords, branch),
            Err(_) => format!("{}/pull/new/{}", remote_url.trim_end_matches('/'), branch),
        };
        Ok(PrInfo {
            url: fallback_url,
            auto_created: false,
        })
    }

    fn test_connection(&self, url: &str) -> Result<ConnectionStatus> {
        crate::providers::shared::test_connection_via_git(url)
    }

    fn compare_url(&self, c: &RepoCoords, branch: &str) -> String {
        format!(
            "https://{}/{}/{}/pull/new/{}",
            c.host, c.owner, c.repo, branch
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_https() {
        let p = GitHubProvider::new(false);
        let c = p.parse_url("https://github.com/org-a/skills.git").unwrap();
        assert_eq!(c.host, "github.com");
        assert_eq!(c.owner, "org-a");
        assert_eq!(c.repo, "skills");
    }

    #[test]
    fn parses_github_ssh() {
        let p = GitHubProvider::new(false);
        let c = p.parse_url("git@github.com:org-a/skills.git").unwrap();
        assert_eq!(c.owner, "org-a");
        assert_eq!(c.repo, "skills");
    }

    #[test]
    fn parses_ghe_custom_domain() {
        let p = GitHubProvider::new(true);
        let c = p
            .parse_url("https://github.acme.internal/org/repo.git")
            .unwrap();
        assert_eq!(c.host, "github.acme.internal");
    }

    #[test]
    fn parses_ghe_cloud_subdomain() {
        let p = GitHubProvider::new(true);
        let c = p.parse_url("https://acme.ghe.com/org/repo").unwrap();
        assert_eq!(c.host, "acme.ghe.com");
    }

    #[test]
    fn rejects_malformed() {
        let p = GitHubProvider::new(false);
        assert!(p.parse_url("not-a-url").is_err());
        assert!(p.parse_url("https://github.com/onlyowner").is_err());
    }

    #[test]
    fn compare_url_shape() {
        let p = GitHubProvider::new(false);
        let c = RepoCoords {
            host: "github.com".into(),
            owner: "o".into(),
            repo: "r".into(),
            url: "...".into(),
        };
        assert_eq!(
            p.compare_url(&c, "feat/x"),
            "https://github.com/o/r/pull/new/feat/x"
        );
    }
}
