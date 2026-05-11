//! TOML-ingestion path for `quay profile add <name> --from-toml <path|->`.
//!
//! Parses a TOML document whose schema mirrors the on-disk `[profiles.<name>]`
//! section and converts it into a [`ProfileDraft`] ready for persistence.

use quay_core::{
    detect_kind_from_url, ProfileDraft, ProviderKind, PushMode, QuayError, RemoteDraft,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Read as _;

// ---------------------------------------------------------------------------
// On-wire TOML shape (mirrors the ingest spec)
// ---------------------------------------------------------------------------

/// The TOML document accepted by `--from-toml`.
///
/// ```toml
/// email = "you@example.com"
///
/// [remotes.work]
/// url = "git@github.com:org/skills.git"
/// provider = "github"   # optional, auto-detected if absent
/// push_mode = "pr"      # optional, defaults to "pr"
/// direct_branch = "develop"  # optional, target branch for direct-mode pushes (omit = default branch)
/// default = true        # optional, defaults to false
/// ```
#[derive(Debug, Deserialize)]
struct IngestDoc {
    #[serde(default)]
    email: String,
    #[serde(default)]
    remotes: BTreeMap<String, IngestRemote>,
}

#[derive(Debug, Deserialize)]
struct IngestRemote {
    url: String,
    /// Optional explicit provider; auto-detected from URL when absent.
    #[serde(default)]
    provider: Option<IngestProvider>,
    #[serde(default)]
    push_mode: IngestPushMode,
    /// Target branch for direct-mode pushes. None = hub's default branch.
    #[serde(default)]
    direct_branch: Option<String>,
    #[serde(default)]
    default: bool,
}

/// Serde-friendly provider names for the ingest TOML.
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum IngestProvider {
    Github,
    Githubenterprise,
    Gitlab,
    Bitbucket,
    Azuredevops,
}

impl From<IngestProvider> for ProviderKind {
    fn from(p: IngestProvider) -> Self {
        match p {
            IngestProvider::Github => ProviderKind::GitHub,
            IngestProvider::Githubenterprise => ProviderKind::GitHubEnterprise,
            IngestProvider::Gitlab => ProviderKind::GitLab,
            IngestProvider::Bitbucket => ProviderKind::Bitbucket,
            IngestProvider::Azuredevops => ProviderKind::AzureDevOps,
        }
    }
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum IngestPushMode {
    #[default]
    Pr,
    Direct,
}

impl From<IngestPushMode> for PushMode {
    fn from(m: IngestPushMode) -> Self {
        match m {
            IngestPushMode::Pr => PushMode::Pr,
            IngestPushMode::Direct => PushMode::Direct,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read TOML text from `arg`: `"-"` reads stdin; anything else reads a file.
pub fn read_from_arg(arg: &str) -> Result<String, QuayError> {
    if arg == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|source| QuayError::Io {
                path: "<stdin>".into(),
                source,
            })?;
        Ok(buf)
    } else {
        std::fs::read_to_string(arg).map_err(|source| QuayError::Io {
            path: arg.into(),
            source,
        })
    }
}

/// Parse `toml_text` (content from a file or stdin) into a [`ProfileDraft`].
///
/// The profile `name` comes from the CLI positional argument, not from the
/// TOML itself (the file contains profile *contents*, not metadata).
///
/// `activate` is passed through from the `--activate` CLI flag.
pub fn parse(
    toml_text: &str,
    profile_name: &str,
    activate: bool,
) -> Result<ProfileDraft, QuayError> {
    let doc: IngestDoc = toml::from_str(toml_text).map_err(|e| QuayError::InvalidConfig {
        path: "<--from-toml input>".into(),
        reason: e.to_string(),
    })?;

    let mut remotes: Vec<RemoteDraft> = Vec::new();
    for (rname, r) in &doc.remotes {
        let provider: ProviderKind = r
            .provider
            .map(ProviderKind::from)
            .unwrap_or_else(|| detect_kind_from_url(&r.url));
        remotes.push(RemoteDraft {
            name: rname.clone(),
            url: r.url.clone(),
            provider,
            push_mode: r.push_mode.into(),
            direct_branch: r.direct_branch.clone(),
            default: r.default,
        });
    }

    Ok(ProfileDraft {
        name: profile_name.to_string(),
        email: doc.email,
        remotes,
        activate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_email_only() {
        let toml = r#"email = "ci@example.com""#;
        let draft = parse(toml, "ci", false).unwrap();
        assert_eq!(draft.name, "ci");
        assert_eq!(draft.email, "ci@example.com");
        assert!(draft.remotes.is_empty());
        assert!(!draft.activate);
    }

    #[test]
    fn parse_with_explicit_provider_and_push_mode() {
        let toml = r#"
            email = "demo@example.com"
            [remotes.azure]
            url = "git@ssh.dev.azure.com:v3/org/proj/repo"
            provider = "azuredevops"
            push_mode = "direct"
            default = true
        "#;
        let draft = parse(toml, "demo", true).unwrap();
        assert_eq!(draft.email, "demo@example.com");
        assert_eq!(draft.remotes.len(), 1);
        let r = &draft.remotes[0];
        assert_eq!(r.name, "azure");
        assert_eq!(r.provider, ProviderKind::AzureDevOps);
        assert_eq!(r.push_mode, PushMode::Direct);
        assert!(r.default);
        assert!(draft.activate);
    }

    #[test]
    fn parse_auto_detects_provider_when_absent() {
        let toml = r#"
            email = "user@example.com"
            [remotes.gh]
            url = "git@github.com:org/skills.git"
        "#;
        let draft = parse(toml, "work", false).unwrap();
        assert_eq!(draft.remotes[0].provider, ProviderKind::GitHub);
        // push_mode defaults to Pr.
        assert_eq!(draft.remotes[0].push_mode, PushMode::Pr);
        // default defaults to false.
        assert!(!draft.remotes[0].default);
    }

    #[test]
    fn parse_rejects_invalid_toml() {
        let err = parse("not valid toml !!!{{", "ci", false).unwrap_err();
        assert!(
            matches!(err, QuayError::InvalidConfig { .. }),
            "expected InvalidConfig, got {err:?}"
        );
    }
}
