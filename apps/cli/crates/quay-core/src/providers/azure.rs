//! Azure DevOps Services provider.
//!
//! Handles three URL shapes:
//! - `https://dev.azure.com/<org>/<project>/_git/<repo>`
//! - `https://<org>.visualstudio.com/<project>/_git/<repo>` (legacy)
//! - `git@ssh.dev.azure.com:v3/<org>/<project>/<repo>` (SSH)
//!
//! `compare_url` always uses `dev.azure.com` (Azure redirects legacy hosts).

use crate::error::{QuayError, Result};
use crate::provider::{ConnectionStatus, PrInfo, Provider, ProviderKind, RepoCoords};
use crate::providers::shared::{cli_available, origin_url, strip_scheme_and_user};
use std::path::Path;
use std::process::Command;

/// Provider implementation for Azure DevOps Services.
pub struct AzureDevOpsProvider;

impl Provider for AzureDevOpsProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::AzureDevOps
    }

    fn parse_url(&self, url: &str) -> Result<RepoCoords> {
        let (host, path) = strip_scheme_and_user(url)?;
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        let (owner, repo) = if host == "dev.azure.com" {
            // [org, project, _git, repo]
            if segments.len() != 4 || segments[2] != "_git" {
                return Err(QuayError::InvalidInput(format!(
                    "azure url '{}' bad shape",
                    url
                )));
            }
            (
                format!("{}/{}", segments[0], segments[1]),
                segments[3].to_string(),
            )
        } else if host.ends_with(".visualstudio.com") {
            // [project, _git, repo] — org is in the host
            let org = host.trim_end_matches(".visualstudio.com");
            if segments.len() != 3 || segments[1] != "_git" {
                return Err(QuayError::InvalidInput(format!(
                    "azure url '{}' bad shape",
                    url
                )));
            }
            (format!("{}/{}", org, segments[0]), segments[2].to_string())
        } else if host == "ssh.dev.azure.com" {
            // [v3, org, project, repo]
            if segments.len() != 4 || segments[0] != "v3" {
                return Err(QuayError::InvalidInput(format!(
                    "azure url '{}' bad shape",
                    url
                )));
            }
            (
                format!("{}/{}", segments[1], segments[2]),
                segments[3].to_string(),
            )
        } else {
            return Err(QuayError::InvalidInput(format!(
                "azure url '{}' unknown host",
                url
            )));
        };

        Ok(RepoCoords {
            host: host.to_string(),
            owner,
            repo,
            url: url.into(),
        })
    }

    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        if !cli_available("az") {
            let coords = self.parse_url(&origin_url(repo)?)?;
            return Ok(PrInfo {
                url: self.compare_url(&coords, branch),
                auto_created: false,
            });
        }
        let out = Command::new("az")
            .args([
                "repos",
                "pr",
                "create",
                "--source-branch",
                branch,
                "--title",
                title,
                "--description",
                body,
            ])
            .current_dir(repo)
            .output()
            .map_err(|e| QuayError::Io {
                path: "az".into(),
                source: e,
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(QuayError::ConfigValidation(format!(
                "az repos pr create failed: {}",
                stderr
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let url = serde_json::from_str::<serde_json::Value>(&stdout)
            .ok()
            .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()))
            .unwrap_or(stdout);
        Ok(PrInfo {
            url,
            auto_created: true,
        })
    }

    fn test_connection(&self, url: &str) -> Result<ConnectionStatus> {
        crate::providers::shared::test_connection_via_git(url)
    }

    fn compare_url(&self, c: &RepoCoords, branch: &str) -> String {
        // Always uses dev.azure.com — Azure redirects legacy visualstudio.com URLs.
        format!(
            "https://dev.azure.com/{}/_git/{}/pullrequestcreate?sourceRef={}",
            c.owner, c.repo, branch
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_azure_modern() {
        let c = AzureDevOpsProvider
            .parse_url("https://dev.azure.com/org/project/_git/repo")
            .unwrap();
        assert_eq!(c.host, "dev.azure.com");
        assert_eq!(c.owner, "org/project");
        assert_eq!(c.repo, "repo");
    }

    #[test]
    fn parses_azure_legacy_visualstudio() {
        let c = AzureDevOpsProvider
            .parse_url("https://org.visualstudio.com/project/_git/repo")
            .unwrap();
        assert_eq!(c.owner, "org/project");
        assert_eq!(c.repo, "repo");
    }

    #[test]
    fn parses_azure_ssh() {
        let c = AzureDevOpsProvider
            .parse_url("git@ssh.dev.azure.com:v3/org/project/repo")
            .unwrap();
        assert_eq!(c.owner, "org/project");
        assert_eq!(c.repo, "repo");
    }

    #[test]
    fn rejects_azure_without_git_segment() {
        assert!(AzureDevOpsProvider
            .parse_url("https://dev.azure.com/org/project/repo")
            .is_err());
    }

    #[test]
    fn compare_url_uses_pullrequestcreate() {
        let c = RepoCoords {
            host: "dev.azure.com".into(),
            owner: "org/proj".into(),
            repo: "r".into(),
            url: "".into(),
        };
        assert!(AzureDevOpsProvider
            .compare_url(&c, "feat/x")
            .contains("pullrequestcreate?sourceRef=feat/x"));
    }
}
