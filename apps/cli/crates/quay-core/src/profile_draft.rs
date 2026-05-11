//! In-progress profile representation shared by all three creation flows:
//! the interactive wizard, TOML ingestion, and explicit CLI flags.
//!
//! Three flows build a [`ProfileDraft`] and call
//! [`ProfileDraft::write_to_user_config`]; that single function is the
//! canonical persistence path so any bug fix applies everywhere.

use crate::config::{ProfileFile, PushMode, RemoteConfig, UserConfigFile};
use crate::error::{QuayError, Result};
use crate::provider::ProviderKind;
use std::collections::BTreeMap;
use std::path::Path;

/// An in-progress remote being assembled before persistence.
#[derive(Debug, Clone)]
pub struct RemoteDraft {
    /// Logical name within the profile (e.g. `"azure"`, `"github"`).
    pub name: String,
    /// Git remote URL.
    pub url: String,
    /// Provider kind — auto-detected from URL, optionally overridden.
    pub provider: ProviderKind,
    /// How `quay push` delivers skills; defaults to `Pr`.
    pub push_mode: PushMode,
    /// Target branch for `push_mode = Direct`. `None` = use hub's default branch.
    pub direct_branch: Option<String>,
    /// Whether this is the default remote for the profile.
    pub default: bool,
}

/// An in-progress profile assembled from any input flow (wizard, TOML, flags).
#[derive(Debug, Clone)]
pub struct ProfileDraft {
    /// Profile name, validated as `^[a-z0-9][a-z0-9_-]*$`.
    pub name: String,
    /// Author email.
    pub email: String,
    /// Remotes to seed into the profile.
    pub remotes: Vec<RemoteDraft>,
    /// If `true`, set `active_profile` to `name` after writing.
    pub activate: bool,
}

impl ProfileDraft {
    /// Persist this draft into the user config file at `path`.
    ///
    /// - Loads the existing file (or starts from a default if it does not exist).
    /// - Inserts the profile; returns [`QuayError::AlreadyExists`] if a profile
    ///   with the same name is already present.
    /// - If `activate` is `true`, sets `active_profile = name`.
    /// - Writes back atomically.
    pub fn write_to_user_config(&self, path: &Path) -> Result<()> {
        let mut file: UserConfigFile = if path.exists() {
            let text = std::fs::read_to_string(path).map_err(|source| QuayError::Io {
                path: path.display().to_string(),
                source,
            })?;
            let mut f: UserConfigFile =
                toml::from_str(&text).map_err(|e| QuayError::InvalidConfig {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                })?;
            f.migrate_legacy_in_place();
            f
        } else {
            UserConfigFile::default()
        };

        if file.profiles.contains_key(&self.name) {
            return Err(QuayError::AlreadyExists(format!("profile '{}'", self.name)));
        }

        let mut remotes: BTreeMap<String, RemoteConfig> = BTreeMap::new();
        for rd in &self.remotes {
            remotes.insert(
                rd.name.clone(),
                RemoteConfig {
                    url: rd.url.clone(),
                    default: rd.default,
                    provider: Some(rd.provider),
                    push_mode: rd.push_mode,
                    direct_branch: rd.direct_branch.clone(),
                },
            );
        }

        let profile = ProfileFile {
            user: crate::config::UserSection {
                name: None,
                email: if self.email.is_empty() {
                    None
                } else {
                    Some(self.email.clone())
                },
            },
            remotes,
            install: None,
        };

        file.profiles.insert(self.name.clone(), profile);

        if self.activate || file.active_profile.is_none() {
            file.active_profile = Some(self.name.clone());
        }

        write_user_file_atomic(path, &file)
    }
}

/// Atomically write a [`UserConfigFile`] to `path`.
fn write_user_file_atomic(path: &Path, file: &UserConfigFile) -> Result<()> {
    let text = toml::to_string_pretty(file).map_err(|e| QuayError::InvalidConfig {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &text).map_err(|source| QuayError::Io {
        path: tmp.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| QuayError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_to_user_config_creates_new_profile_section_with_remotes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let draft = ProfileDraft {
            name: "demo".into(),
            email: "demo@example.com".into(),
            remotes: vec![RemoteDraft {
                name: "azure".into(),
                url: "git@ssh.dev.azure.com:v3/org/proj/repo".into(),
                provider: ProviderKind::AzureDevOps,
                push_mode: PushMode::Direct,
                direct_branch: None,
                default: true,
            }],
            activate: true,
        };
        draft.write_to_user_config(&path).unwrap();

        let txt = std::fs::read_to_string(&path).unwrap();
        assert!(
            txt.contains("active_profile = \"demo\""),
            "missing active: {txt}"
        );
        // TOML serialises nested tables inline; section header includes sub-keys.
        assert!(
            txt.contains("profiles.demo"),
            "missing profile header: {txt}"
        );
        assert!(
            txt.contains("email = \"demo@example.com\""),
            "missing email: {txt}"
        );
        assert!(
            txt.contains("[profiles.demo.remotes.azure]"),
            "missing remote section: {txt}"
        );
        assert!(
            txt.contains("provider = \"azuredevops\""),
            "missing provider: {txt}"
        );
        assert!(
            txt.contains("push_mode = \"direct\""),
            "missing push_mode: {txt}"
        );
    }

    #[test]
    fn write_to_user_config_rejects_duplicate_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        // Write once.
        let draft = ProfileDraft {
            name: "demo".into(),
            email: "demo@example.com".into(),
            remotes: vec![],
            activate: true,
        };
        draft.write_to_user_config(&path).unwrap();

        // Second write with the same name must fail.
        let err = draft.write_to_user_config(&path).unwrap_err();
        assert!(
            matches!(err, QuayError::AlreadyExists(_)),
            "expected AlreadyExists, got {err:?}"
        );
    }

    #[test]
    fn write_to_user_config_sets_active_when_flag_true() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let draft = ProfileDraft {
            name: "work".into(),
            email: "w@work.com".into(),
            remotes: vec![],
            activate: true,
        };
        draft.write_to_user_config(&path).unwrap();
        let txt = std::fs::read_to_string(&path).unwrap();
        assert!(txt.contains("active_profile = \"work\""));
    }

    #[test]
    fn write_to_user_config_does_not_clobber_existing_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        // First profile.
        ProfileDraft {
            name: "first".into(),
            email: "f@x.com".into(),
            remotes: vec![],
            activate: true,
        }
        .write_to_user_config(&path)
        .unwrap();

        // Second profile.
        ProfileDraft {
            name: "second".into(),
            email: "s@x.com".into(),
            remotes: vec![],
            activate: false,
        }
        .write_to_user_config(&path)
        .unwrap();

        let txt = std::fs::read_to_string(&path).unwrap();
        // TOML serialises nested tables with sub-key headers like [profiles.first.user].
        assert!(txt.contains("profiles.first"), "first profile gone: {txt}");
        assert!(
            txt.contains("profiles.second"),
            "second profile missing: {txt}"
        );
        // active_profile should still be "first" since activate=false.
        assert!(
            txt.contains("active_profile = \"first\""),
            "active changed: {txt}"
        );
    }
}
