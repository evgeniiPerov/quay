//! GitLab provider — supports gitlab.com and self-hosted instances,
//! including nested subgroups.

use crate::error::{QuayError, Result};
use crate::provider::{ConnectionStatus, PrInfo, Provider, ProviderKind, RepoCoords};
use crate::providers::shared::{cli_available, origin_url, strip_scheme_and_user};
use std::path::Path;
use std::process::Command;

/// Provider implementation for GitLab (gitlab.com and self-hosted).
pub struct GitLabProvider;

impl Provider for GitLabProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::GitLab
    }

    fn parse_url(&self, url: &str) -> Result<RepoCoords> {
        let (host, path) = strip_scheme_and_user(url)?;
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() < 2 {
            return Err(QuayError::InvalidInput(format!(
                "gitlab url '{}' missing owner/repo",
                url
            )));
        }
        let repo = segments.pop().unwrap().to_string();
        let owner = segments.join("/");
        Ok(RepoCoords {
            host: host.to_string(),
            owner,
            repo,
            url: url.into(),
        })
    }

    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        if !cli_available("glab") {
            let coords = self.parse_url(&origin_url(repo)?)?;
            return Ok(PrInfo {
                url: self.compare_url(&coords, branch),
                auto_created: false,
            });
        }
        let out = Command::new("glab")
            .arg("-R")
            .arg(repo)
            .args([
                "mr",
                "create",
                "--source-branch",
                branch,
                "--title",
                title,
                "--description",
                body,
            ])
            .output()
            .map_err(|e| QuayError::Io {
                path: "glab".into(),
                source: e,
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(QuayError::ConfigValidation(format!(
                "glab mr create failed: {}",
                stderr
            )));
        }
        Ok(PrInfo {
            url: String::from_utf8_lossy(&out.stdout).trim().into(),
            auto_created: true,
        })
    }

    fn test_connection(&self, url: &str) -> Result<ConnectionStatus> {
        crate::providers::shared::test_connection_via_git(url)
    }

    fn compare_url(&self, c: &RepoCoords, branch: &str) -> String {
        format!(
            "https://{}/{}/{}/-/merge_requests/new?merge_request[source_branch]={}",
            c.host, c.owner, c.repo, branch
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gitlab_com_simple() {
        let c = GitLabProvider
            .parse_url("https://gitlab.com/org-a/skills.git")
            .unwrap();
        assert_eq!(c.host, "gitlab.com");
        assert_eq!(c.owner, "org-a");
        assert_eq!(c.repo, "skills");
    }

    #[test]
    fn parses_gitlab_subgroup() {
        let c = GitLabProvider
            .parse_url("https://gitlab.com/org-a/team/skills.git")
            .unwrap();
        assert_eq!(c.owner, "org-a/team");
        assert_eq!(c.repo, "skills");
    }

    #[test]
    fn parses_gitlab_deeply_nested() {
        let c = GitLabProvider
            .parse_url("git@gitlab.example.com:foo/bar/baz/qux.git")
            .unwrap();
        assert_eq!(c.host, "gitlab.example.com");
        assert_eq!(c.owner, "foo/bar/baz");
        assert_eq!(c.repo, "qux");
    }

    #[test]
    fn rejects_too_few_segments() {
        assert!(GitLabProvider
            .parse_url("https://gitlab.com/onlyowner")
            .is_err());
    }

    #[test]
    fn compare_url_uses_merge_requests() {
        let c = RepoCoords {
            host: "gitlab.com".into(),
            owner: "o/t".into(),
            repo: "r".into(),
            url: "".into(),
        };
        let url = GitLabProvider.compare_url(&c, "feat/x");
        assert!(url.contains("/-/merge_requests/new"));
        assert!(url.contains("source_branch"));
    }
}
