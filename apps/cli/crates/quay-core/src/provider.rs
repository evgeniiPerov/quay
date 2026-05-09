//! PR/MR opener abstraction and provider trait.
//!
//! [`PrOpener`] is the legacy thin trait kept for back-compat with [`crate::pusher`].
//! [`Provider`] is the richer abstraction introduced in Plan 7a.  A blanket
//! `impl<T: Provider> PrOpener for T` means every [`Provider`] implementation
//! automatically satisfies [`PrOpener`] without any extra boilerplate.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Identifies which hosting provider backs a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    GitHub,
    GitHubEnterprise,
    GitLab,
    Bitbucket,
    AzureDevOps,
}

/// Parsed coordinates extracted from a remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCoords {
    pub host: String,
    pub owner: String,
    pub repo: String,
    /// The original URL that was parsed.
    pub url: String,
}

/// Outcome of a live test-connection probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Connection succeeded and `registry.json` was found.
    Ok { registry_size_bytes: u64 },
    /// Host is unreachable (DNS / network failure).
    Unreachable(String),
    /// Authentication rejected (SSH key, token, etc.).
    AuthFailed(String),
    /// Connected, but `registry.json` was not found at HEAD.
    NoRegistry(String),
}

/// Result of a successful PR/MR open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    /// User-visible URL (PR page, or hint to open one manually).
    pub url: String,
    /// True when the PR was created automatically; false when the caller still
    /// needs to open the PR/MR by hand (printed URL is a hint, not a link to a real PR).
    pub auto_created: bool,
}

// ── Provider trait ─────────────────────────────────────────────────────────────

/// Abstraction over a git-hosting provider.
///
/// All methods are provided by the concrete implementation;
/// `quay-core` includes [`GhCliOpener`] (GitHub) as a built-in.
/// Additional providers live in `crate::providers::{github,gitlab,bitbucket,azure}`.
pub trait Provider: Send + Sync {
    /// Which kind this provider represents.
    fn kind(&self) -> ProviderKind;

    /// Parse a remote URL into structured coordinates.
    fn parse_url(&self, url: &str) -> Result<RepoCoords>;

    /// Open (or prepare the URL for) a pull request / merge request.
    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo>;

    /// Probe reachability, authentication, and registry presence.
    fn test_connection(&self, url: &str) -> Result<ConnectionStatus>;

    /// Build the provider-specific URL for creating a PR/MR from `branch`.
    fn compare_url(&self, coords: &RepoCoords, branch: &str) -> String;
}

// ── PrOpener (legacy back-compat trait) ──────────────────────────────────────

/// Opens a PR/MR after a branch has been pushed.
///
/// This thin trait is kept so [`crate::pusher::SkillPusher`] does not need
/// cascading changes when the provider abstraction was introduced.  The
/// blanket impl below means any [`Provider`] automatically satisfies it.
pub trait PrOpener {
    /// Open (or prepare the URL for) a pull request on the hosting provider.
    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo>;
}

/// Blanket: every [`Provider`] is also a [`PrOpener`].
impl<T: Provider> PrOpener for T {
    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        Provider::open_pr(self, repo, branch, title, body)
    }
}

/// Allow `Box<dyn Provider>` to be used wherever `PrOpener` is required.
///
/// This enables callers that hold a factory-resolved `Box<dyn Provider>` to pass a
/// reference to it as `&dyn PrOpener` (or as `P: PrOpener`) without any extra
/// wrapping — useful in `commands::push` where the provider is resolved from the
/// remote config before constructing [`crate::pusher::SkillPusher`].
impl PrOpener for Box<dyn Provider> {
    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        (**self).open_pr(repo, branch, title, body)
    }
}

// ── Factory ───────────────────────────────────────────────────────────────────

/// Return the right [`Provider`] for `url`, optionally overriding auto-detect.
///
/// When `explicit` is `Some`, it always wins.  Otherwise [`detect_kind_from_url`]
/// is called.  The `QUAY_PROVIDER` env-var escape hatch was removed in Plan 7a —
/// use the `provider` field in the remote's TOML entry (or `--provider`) instead.
pub fn provider_for_remote(url: &str, explicit: Option<ProviderKind>) -> Box<dyn Provider> {
    let kind = explicit.unwrap_or_else(|| detect_kind_from_url(url));
    match kind {
        ProviderKind::GitHub => Box::new(crate::providers::github::GitHubProvider::new(false)),
        ProviderKind::GitHubEnterprise => {
            Box::new(crate::providers::github::GitHubProvider::new(true))
        }
        ProviderKind::GitLab => Box::new(crate::providers::gitlab::GitLabProvider),
        ProviderKind::Bitbucket => Box::new(crate::providers::bitbucket::BitbucketProvider),
        ProviderKind::AzureDevOps => Box::new(crate::providers::azure::AzureDevOpsProvider),
    }
}

/// Infer the provider kind from a remote URL using substring matching.
///
/// Detection rules (in priority order):
/// 1. `github.com`       → [`ProviderKind::GitHub`]
/// 2. `gitlab.com`       → [`ProviderKind::GitLab`]
/// 3. `bitbucket.org`    → [`ProviderKind::Bitbucket`]
/// 4. `dev.azure.com`    → [`ProviderKind::AzureDevOps`]
/// 5. `visualstudio.com` → [`ProviderKind::AzureDevOps`]
/// 6. `gitlab.`          → [`ProviderKind::GitLab`] (self-hosted)
/// 7. `ghe.com`          → [`ProviderKind::GitHubEnterprise`] (GHE Cloud subdomain)
/// 8. `github.`          → [`ProviderKind::GitHubEnterprise`] (on-prem / custom domain)
/// 9. fallback           → [`ProviderKind::GitHub`]
pub fn detect_kind_from_url(url: &str) -> ProviderKind {
    let lower = url.to_ascii_lowercase();
    if lower.contains("github.com") {
        ProviderKind::GitHub
    } else if lower.contains("gitlab.com") {
        ProviderKind::GitLab
    } else if lower.contains("bitbucket.org") {
        ProviderKind::Bitbucket
    } else if lower.contains("dev.azure.com") || lower.contains("visualstudio.com") {
        ProviderKind::AzureDevOps
    } else if lower.contains("gitlab.") {
        ProviderKind::GitLab
    } else if lower.contains("ghe.com") || lower.contains("github.") {
        ProviderKind::GitHubEnterprise
    } else {
        ProviderKind::GitHub
    }
}

// ── GhCliOpener ───────────────────────────────────────────────────────────────

/// Back-compat shim for callers that still hold a `GhCliOpener` directly.
///
/// All five [`Provider`] methods delegate to
/// [`GitHubProvider::new(false)`](crate::providers::github::GitHubProvider).
/// New code should use [`provider_for_remote`] instead.
pub struct GhCliOpener;

impl Default for GhCliOpener {
    fn default() -> Self {
        Self
    }
}

impl Provider for GhCliOpener {
    fn kind(&self) -> ProviderKind {
        ProviderKind::GitHub
    }

    fn parse_url(&self, url: &str) -> Result<RepoCoords> {
        crate::providers::github::GitHubProvider::new(false).parse_url(url)
    }

    fn open_pr(&self, repo: &Path, branch: &str, title: &str, body: &str) -> Result<PrInfo> {
        Provider::open_pr(
            &crate::providers::github::GitHubProvider::new(false),
            repo,
            branch,
            title,
            body,
        )
    }

    fn test_connection(&self, url: &str) -> Result<ConnectionStatus> {
        crate::providers::github::GitHubProvider::new(false).test_connection(url)
    }

    fn compare_url(&self, coords: &RepoCoords, branch: &str) -> String {
        crate::providers::github::GitHubProvider::new(false).compare_url(coords, branch)
    }
}

// ── Test-only opener ──────────────────────────────────────────────────────────

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

// ── Tests ─────────────────────────────────────────────────────────────────────

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

    // ── detect_kind_from_url ──────────────────────────────────────────────────

    #[test]
    fn detect_github_com() {
        assert_eq!(
            detect_kind_from_url("https://github.com/o/r.git"),
            ProviderKind::GitHub
        );
    }

    #[test]
    fn detect_ghe_subdomain() {
        assert_eq!(
            detect_kind_from_url("https://acme.ghe.com/o/r.git"),
            ProviderKind::GitHubEnterprise
        );
    }

    #[test]
    fn detect_ghe_custom_domain() {
        assert_eq!(
            detect_kind_from_url("https://github.acme.internal/o/r.git"),
            ProviderKind::GitHubEnterprise
        );
    }

    #[test]
    fn detect_gitlab_com() {
        assert_eq!(
            detect_kind_from_url("git@gitlab.com:o/r.git"),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn detect_self_hosted_gitlab() {
        assert_eq!(
            detect_kind_from_url("https://gitlab.example.com/o/r.git"),
            ProviderKind::GitLab
        );
    }

    #[test]
    fn detect_bitbucket() {
        assert_eq!(
            detect_kind_from_url("https://bitbucket.org/o/r.git"),
            ProviderKind::Bitbucket
        );
    }

    #[test]
    fn detect_azure_modern() {
        assert_eq!(
            detect_kind_from_url("https://dev.azure.com/org/project/_git/repo"),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn detect_azure_legacy() {
        assert_eq!(
            detect_kind_from_url("https://org.visualstudio.com/project/_git/repo"),
            ProviderKind::AzureDevOps
        );
    }

    #[test]
    fn factory_honors_explicit_kind() {
        let p = provider_for_remote("https://github.com/o/r.git", Some(ProviderKind::GitLab));
        assert_eq!(p.kind(), ProviderKind::GitLab);
    }
}
