use crate::error::{QuayError, Result};
use crate::provider::ProviderKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How `quay push` delivers a skill to the hub.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushMode {
    /// Open a pull request against the hub's default branch (the historical default).
    #[default]
    Pr,
    /// Commit on the hub's default branch locally and `git push` it directly.
    /// No PR; provider CLI is not invoked. Works on any git server.
    Direct,
}

/// Merged configuration after combining user-level and project-level files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub user: UserSection,
    #[serde(default)]
    pub remotes: BTreeMap<String, RemoteConfig>,
    #[serde(default)]
    pub install: InstallConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSection {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub url: String,
    #[serde(default)]
    pub default: bool,
    /// Explicit provider override.  When `None`, the provider is auto-detected
    /// from the URL by [`crate::provider::detect_kind_from_url`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    /// How `quay push` delivers a skill to the hub. New in Plan 9.
    /// Defaults to `Pr` when reading old configs.
    #[serde(default)]
    pub push_mode: PushMode,
}

/// Persisted metadata about the quay installation (e.g. whether first-run
/// onboarding has been completed). Absent or default values are omitted from
/// disk so that existing config files continue to round-trip cleanly.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetaSection {
    /// Set to `true` once the user has completed (or explicitly skipped) the
    /// first-run onboarding screen.
    #[serde(default, skip_serializing_if = "is_default_false")]
    pub onboarded: bool,
}

fn is_default_false(b: &bool) -> bool {
    !*b
}

impl MetaSection {
    fn is_default(&self) -> bool {
        !self.onboarded
    }
}

/// On-disk shape of the user config file. Supports both the legacy flat layout
/// (Plan 1 — `[user]` + top-level `[remotes.*]`) and the new profile-aware
/// layout (`active_profile` + `[profiles.<name>]`). Old-shape files
/// deserialize as a single profile named `"personal"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserConfigFile {
    /// Installation metadata; omitted from disk when all values are default.
    #[serde(default, skip_serializing_if = "MetaSection::is_default")]
    pub meta: MetaSection,
    pub active_profile: Option<String>,
    pub profiles: BTreeMap<String, ProfileFile>,
    /// Legacy flat fields, retained for backward compatibility.
    pub user: Option<UserSection>,
    pub remotes: Option<BTreeMap<String, RemoteConfig>>,
}

/// On-disk shape of a single profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileFile {
    pub user: UserSection,
    pub remotes: BTreeMap<String, RemoteConfig>,
    pub install: Option<InstallConfig>,
}

/// On-disk shape of the project config file (`.quay/config.toml`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfigFile {
    /// Optional pin: project requires this named profile.
    pub profile: Option<String>,
    pub install: InstallConfig,
    pub remotes: BTreeMap<String, RemoteConfig>,
    /// Project-level user overrides (e.g. commit author email).
    pub user: UserSection,
}

impl UserConfigFile {
    /// If the file uses the legacy flat shape (no `[profiles.*]` block, but
    /// `[user]` or `[remotes]` present), fold those fields into a synthetic
    /// `"personal"` profile and set `active_profile` accordingly. No-op for
    /// already-migrated files.
    pub fn migrate_legacy_in_place(&mut self) {
        let has_legacy = self.user.is_some() || self.remotes.is_some();
        let has_profiles = !self.profiles.is_empty();
        if has_legacy && !has_profiles {
            let profile = ProfileFile {
                user: self.user.take().unwrap_or_default(),
                remotes: self.remotes.take().unwrap_or_default(),
                install: None,
            };
            self.profiles.insert("personal".into(), profile);
            if self.active_profile.is_none() {
                self.active_profile = Some("personal".into());
            }
        } else {
            // Drop now-redundant legacy fields when the new shape is present.
            self.user = None;
            self.remotes = None;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorStrategy {
    Symlink,
    Junction,
    Copy,
    #[default]
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorConfig {
    /// Mirror destination directory, e.g. `.claude/skills`.
    pub path: PathBuf,
    /// How to project canonical content into the mirror. Defaults to `auto`.
    #[serde(default)]
    pub strategy: MirrorStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallConfig {
    #[serde(default = "default_canonical")]
    pub canonical: PathBuf,
    /// Optional mirror destinations.
    #[serde(default)]
    pub mirrors: Vec<MirrorConfig>,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            canonical: default_canonical(),
            mirrors: Vec::new(),
        }
    }
}

fn default_canonical() -> PathBuf {
    PathBuf::from(".agents/skills")
}

impl Config {
    /// Load and merge user (`~/.config/quay/config.toml`) and project (`.quay/config.toml`).
    /// Project values override user values for non-collection fields; remotes maps are union'd.
    pub fn load(user_path: Option<&Path>, project_path: Option<&Path>) -> Result<Self> {
        let user = match user_path {
            Some(p) if p.exists() => Self::read(p)?,
            _ => Config::default(),
        };
        let project = match project_path {
            Some(p) if p.exists() => Self::read(p)?,
            _ => Config::default(),
        };
        Ok(Self::merge(user, project))
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| QuayError::Io {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str::<Config>(&text).map_err(|e| QuayError::InvalidConfig {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| QuayError::InvalidConfig {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        std::fs::write(path, text).map_err(|source| QuayError::Io {
            path: path.display().to_string(),
            source,
        })
    }

    fn merge(user: Config, project: Config) -> Config {
        let mut remotes = user.remotes;
        for (k, v) in project.remotes {
            remotes.insert(k, v);
        }
        Config {
            user: UserSection {
                name: project.user.name.or(user.user.name),
                email: project.user.email.or(user.user.email),
            },
            remotes,
            install: if project.install != InstallConfig::default() {
                project.install
            } else {
                user.install
            },
        }
    }

    pub fn default_remote(&self) -> Option<(&String, &RemoteConfig)> {
        self.remotes.iter().find(|(_, r)| r.default)
    }
}

impl Config {
    /// Load and resolve a profile-aware config.
    ///
    /// Resolution order (first match wins):
    /// 1. `profile_override` (e.g. CLI `--profile=<name>`).
    /// 2. `QUAY_PROFILE` env var.
    /// 3. Project config's `profile = "<name>"` pin.
    /// 4. User config's `active_profile`.
    /// 5. Single-profile case: if exactly one profile exists, use it.
    /// 6. No profiles in user config: fall back to project-only mode (empty profile).
    /// 7. Multiple profiles and no selection: error (`AmbiguousProfile`).
    pub fn load_resolved(
        user_path: Option<&Path>,
        project_path: Option<&Path>,
        profile_override: Option<&str>,
    ) -> Result<Self> {
        let mut user_file: UserConfigFile = match user_path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p).map_err(|source| QuayError::Io {
                    path: p.display().to_string(),
                    source,
                })?;
                toml::from_str(&text).map_err(|e| QuayError::InvalidConfig {
                    path: p.display().to_string(),
                    reason: e.to_string(),
                })?
            }
            _ => UserConfigFile::default(),
        };
        user_file.migrate_legacy_in_place();

        let project_file: ProjectConfigFile = match project_path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p).map_err(|source| QuayError::Io {
                    path: p.display().to_string(),
                    source,
                })?;
                toml::from_str(&text).map_err(|e| QuayError::InvalidConfig {
                    path: p.display().to_string(),
                    reason: e.to_string(),
                })?
            }
            _ => ProjectConfigFile::default(),
        };

        let env_profile = std::env::var("QUAY_PROFILE").ok().filter(|s| !s.is_empty());
        let resolved_name = profile_override
            .map(str::to_string)
            .or(env_profile)
            .or_else(|| project_file.profile.clone())
            .or_else(|| user_file.active_profile.clone())
            .or_else(|| {
                if user_file.profiles.len() == 1 {
                    // SAFETY: len == 1 guarantees next() returns Some.
                    Some(user_file.profiles.keys().next().unwrap().clone())
                } else {
                    None
                }
            });

        // When no profile name could be resolved and the user file has no profiles at all
        // (e.g. no user config file present), fall back to project-only config so that
        // project-level remotes still work without a user config.
        if resolved_name.is_none() && user_file.profiles.is_empty() {
            let install = if project_file.install != InstallConfig::default() {
                project_file.install
            } else {
                InstallConfig::default()
            };
            return Ok(Config {
                user: project_file.user,
                remotes: project_file.remotes,
                install,
            });
        }

        let name = match resolved_name {
            Some(n) => n,
            None => {
                // profiles is non-empty but no single profile could be selected
                return Err(QuayError::AmbiguousProfile);
            }
        };

        if let Some(pinned) = project_file.profile.as_deref() {
            if !user_file.profiles.contains_key(pinned) {
                return Err(QuayError::ProfileRequired(pinned.into()));
            }
        }

        let profile = user_file
            .profiles
            .get(&name)
            .ok_or_else(|| QuayError::ProfileUnknown(name.clone()))?
            .clone();

        let mut remotes = profile.remotes;
        for (k, v) in project_file.remotes {
            remotes.insert(k, v);
        }
        let install = if project_file.install != InstallConfig::default() {
            project_file.install
        } else {
            profile.install.unwrap_or_default()
        };

        // Project-level [user] overrides profile [user] (same precedence rule as Config::merge).
        let user = UserSection {
            name: project_file.user.name.or(profile.user.name),
            email: project_file.user.email.or(profile.user.email),
        };

        Ok(Config {
            user,
            remotes,
            install,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;

    #[test]
    fn parses_minimal() {
        let toml = r#"
            [user]
            name = "Evgenii"
            email = "e@example.com"

            [remotes.my-hub]
            url = "https://github.com/x/y.git"
            default = true
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.user.email.as_deref(), Some("e@example.com"));
        assert!(cfg.remotes["my-hub"].default);
    }

    #[test]
    fn install_canonical_defaults_when_omitted() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.install.canonical, PathBuf::from(".agents/skills"));
    }

    #[test]
    fn project_overrides_user_identity_and_unions_remotes() {
        let user = Config {
            user: UserSection {
                name: Some("U".into()),
                email: Some("u@x".into()),
            },
            remotes: BTreeMap::from([(
                "user-hub".into(),
                RemoteConfig {
                    url: "u".into(),
                    default: true,
                    provider: None,
                    push_mode: PushMode::default(),
                },
            )]),
            ..Default::default()
        };
        let project = Config {
            user: UserSection {
                name: Some("P".into()),
                email: None,
            },
            remotes: BTreeMap::from([(
                "proj-hub".into(),
                RemoteConfig {
                    url: "p".into(),
                    default: false,
                    provider: None,
                    push_mode: PushMode::default(),
                },
            )]),
            ..Default::default()
        };
        let merged = Config::merge(user, project);
        assert_eq!(merged.user.name.as_deref(), Some("P"));
        assert_eq!(merged.user.email.as_deref(), Some("u@x"));
        assert_eq!(merged.remotes.len(), 2);
        assert!(merged.remotes.contains_key("user-hub"));
        assert!(merged.remotes.contains_key("proj-hub"));
    }

    #[test]
    fn round_trip_through_disk() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.child("c.toml");
        let cfg = Config {
            user: UserSection {
                name: Some("E".into()),
                email: None,
            },
            remotes: BTreeMap::from([(
                "h".into(),
                RemoteConfig {
                    url: "https://x/y.git".into(),
                    default: true,
                    provider: None,
                    push_mode: PushMode::default(),
                },
            )]),
            install: InstallConfig::default(),
        };
        cfg.write(path.path()).unwrap();
        let read = Config::read(path.path()).unwrap();
        assert_eq!(read, cfg);
    }

    #[test]
    fn default_remote_finds_flagged_one() {
        let cfg = Config {
            remotes: BTreeMap::from([
                (
                    "a".into(),
                    RemoteConfig {
                        url: "x".into(),
                        default: false,
                        provider: None,
                        push_mode: PushMode::default(),
                    },
                ),
                (
                    "b".into(),
                    RemoteConfig {
                        url: "y".into(),
                        default: true,
                        provider: None,
                        push_mode: PushMode::default(),
                    },
                ),
            ]),
            ..Default::default()
        };
        assert_eq!(cfg.default_remote().unwrap().0, "b");
    }

    #[test]
    fn user_file_parses_new_nested_shape() {
        let toml = r#"
            active_profile = "work"

            [profiles.work.user]
            name = "Evgenii"
            email = "e@work.example"

            [profiles.work.remotes.frontend]
            url = "https://github.com/work/skills-frontend.git"
            default = true

            [profiles.personal.user]
            email = "e@personal.example"
        "#;
        let mut file: UserConfigFile = toml::from_str(toml).unwrap();
        file.migrate_legacy_in_place(); // no-op
        assert_eq!(file.active_profile.as_deref(), Some("work"));
        assert_eq!(file.profiles.len(), 2);
        assert_eq!(
            file.profiles["work"].user.email.as_deref(),
            Some("e@work.example")
        );
        assert!(file.profiles["work"].remotes.contains_key("frontend"));
    }

    #[test]
    fn user_file_migrates_legacy_flat_shape() {
        let toml = r#"
            [user]
            name = "Evgenii"
            email = "e@old"

            [remotes.my-hub]
            url = "https://github.com/x/y.git"
            default = true
        "#;
        let mut file: UserConfigFile = toml::from_str(toml).unwrap();
        file.migrate_legacy_in_place();
        assert_eq!(file.active_profile.as_deref(), Some("personal"));
        assert_eq!(file.profiles.len(), 1);
        let p = &file.profiles["personal"];
        assert_eq!(p.user.email.as_deref(), Some("e@old"));
        assert!(p.remotes.contains_key("my-hub"));
        assert!(file.user.is_none());
        assert!(file.remotes.is_none());
    }

    #[test]
    fn user_file_with_no_legacy_or_profiles_is_empty() {
        let mut file: UserConfigFile = toml::from_str("").unwrap();
        file.migrate_legacy_in_place();
        assert!(file.profiles.is_empty());
        assert!(file.active_profile.is_none());
    }

    #[test]
    fn project_file_parses_with_profile_pin_and_overlay() {
        let toml = r#"
            profile = "work"

            [install]
            canonical = ".agents/skills"

            [remotes.community]
            url = "https://github.com/community/skills.git"
        "#;
        let proj: ProjectConfigFile = toml::from_str(toml).unwrap();
        assert_eq!(proj.profile.as_deref(), Some("work"));
        assert!(proj.remotes.contains_key("community"));
    }

    fn write_user(dir: &assert_fs::TempDir, contents: &str) -> std::path::PathBuf {
        let p = dir.child("user.toml");
        std::fs::write(p.path(), contents).unwrap();
        p.path().to_path_buf()
    }
    fn write_project(dir: &assert_fs::TempDir, contents: &str) -> std::path::PathBuf {
        let p = dir.child("project.toml");
        std::fs::write(p.path(), contents).unwrap();
        p.path().to_path_buf()
    }

    #[test]
    fn resolves_explicit_override() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                active_profile = "work"
                [profiles.work.user]
                email = "e@work"
                [profiles.personal.user]
                email = "e@home"
            "#,
        );
        let cfg = Config::load_resolved(Some(&user), None, Some("personal")).unwrap();
        assert_eq!(cfg.user.email.as_deref(), Some("e@home"));
    }

    #[test]
    fn resolves_active_profile_when_no_override() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                active_profile = "work"
                [profiles.work.user]
                email = "e@work"
                [profiles.personal.user]
                email = "e@home"
            "#,
        );
        let cfg = Config::load_resolved(Some(&user), None, None).unwrap();
        assert_eq!(cfg.user.email.as_deref(), Some("e@work"));
    }

    #[test]
    fn resolves_single_profile_implicit() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                [profiles.personal.user]
                email = "e@home"
            "#,
        );
        let cfg = Config::load_resolved(Some(&user), None, None).unwrap();
        assert_eq!(cfg.user.email.as_deref(), Some("e@home"));
    }

    #[test]
    fn empty_user_config_falls_back_to_project_only() {
        // An empty (or absent) user config has no profiles. Rather than erroring, we
        // fall through to project-only mode so commands still work before a user config
        // is created.
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(&dir, "");
        let cfg = Config::load_resolved(Some(&user), None, None).unwrap();
        assert!(cfg.remotes.is_empty());
        assert!(cfg.user.email.is_none());
    }

    #[test]
    fn errors_when_ambiguous_with_no_active() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                [profiles.a.user]
                email = "a"
                [profiles.b.user]
                email = "b"
            "#,
        );
        let err = Config::load_resolved(Some(&user), None, None).unwrap_err();
        assert!(matches!(err, QuayError::AmbiguousProfile));
    }

    #[test]
    fn project_pin_must_exist_in_user_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                [profiles.work.user]
                email = "e@work"
            "#,
        );
        let project = write_project(&dir, r#"profile = "missing""#);
        let err = Config::load_resolved(Some(&user), Some(&project), None).unwrap_err();
        assert!(matches!(err, QuayError::ProfileRequired(name) if name == "missing"));
    }

    #[test]
    fn project_overlay_remotes_layer_on_top() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                active_profile = "work"
                [profiles.work.user]
                email = "e@work"
                [profiles.work.remotes.frontend]
                url = "https://x/frontend.git"
            "#,
        );
        let project = write_project(
            &dir,
            r#"
                [remotes.community]
                url = "https://x/community.git"
            "#,
        );
        let cfg = Config::load_resolved(Some(&user), Some(&project), None).unwrap();
        assert_eq!(cfg.remotes.len(), 2);
        assert!(cfg.remotes.contains_key("frontend"));
        assert!(cfg.remotes.contains_key("community"));
    }

    #[test]
    fn legacy_flat_user_file_works_without_changes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                [user]
                email = "old@me"
                [remotes.h]
                url = "https://x/y.git"
            "#,
        );
        let cfg = Config::load_resolved(Some(&user), None, None).unwrap();
        assert_eq!(cfg.user.email.as_deref(), Some("old@me"));
        assert!(cfg.remotes.contains_key("h"));
    }

    // Ignored: mutates the process-global env, which is shared across threads
    // in the same test binary. Running this test in parallel with
    // `errors_when_zero_profiles` (or any other test that calls
    // `load_resolved`) can cause spurious `ProfileUnknown` failures in those
    // tests when they observe QUAY_PROFILE before `remove_var` runs.
    // Run manually with: `cargo test env_var_overrides_active_profile -- --ignored`
    #[test]
    #[ignore]
    fn env_var_overrides_active_profile() {
        let dir = assert_fs::TempDir::new().unwrap();
        let user = write_user(
            &dir,
            r#"
                active_profile = "work"
                [profiles.work.user]
                email = "e@work"
                [profiles.personal.user]
                email = "e@home"
            "#,
        );
        // SAFETY: set + unset around the call. Tests within the same target
        // serialize on this env var; cargo runs different test binaries in
        // separate processes so cross-binary races are not a concern here.
        // The test is `#[ignore]` so single-threaded `--ignored` runs avoid the
        // intra-binary race entirely.
        unsafe {
            std::env::set_var("QUAY_PROFILE", "personal");
        }
        let cfg = Config::load_resolved(Some(&user), None, None).unwrap();
        unsafe {
            std::env::remove_var("QUAY_PROFILE");
        }
        assert_eq!(cfg.user.email.as_deref(), Some("e@home"));
    }

    #[test]
    fn install_with_mirrors_parses() {
        let toml = r#"
            [install]
            canonical = ".agents/skills"
            mirrors = [
              { path = ".claude/skills", strategy = "auto" },
              { path = ".codex/skills",  strategy = "copy" },
            ]
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.install.mirrors.len(), 2);
        assert_eq!(cfg.install.mirrors[0].path, PathBuf::from(".claude/skills"));
        assert_eq!(cfg.install.mirrors[0].strategy, MirrorStrategy::Auto);
        assert_eq!(cfg.install.mirrors[1].strategy, MirrorStrategy::Copy);
    }

    #[test]
    fn install_without_mirrors_defaults_to_empty() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.install.mirrors.is_empty());
    }

    #[test]
    fn mirror_strategy_default_is_auto_when_omitted() {
        let toml = r#"
            [install]
            mirrors = [{ path = ".claude/skills" }]
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.install.mirrors[0].strategy, MirrorStrategy::Auto);
    }

    #[test]
    fn mirror_config_round_trips() {
        let cfg = InstallConfig {
            canonical: PathBuf::from(".agents/skills"),
            mirrors: vec![MirrorConfig {
                path: PathBuf::from(".claude/skills"),
                strategy: MirrorStrategy::Symlink,
            }],
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let parsed: InstallConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn loads_pre_7a_remote_without_provider() {
        let toml = r#"url = "git@github.com:o/r.git"
default = true
"#;
        let r: RemoteConfig = toml::from_str(toml).unwrap();
        assert!(r.provider.is_none());
        assert!(r.default);
    }

    #[test]
    fn round_trip_omits_default_provider() {
        let r = RemoteConfig {
            url: "git@x:o/r.git".into(),
            default: false,
            provider: None,
            push_mode: PushMode::default(),
        };
        let s = toml::to_string(&r).unwrap();
        assert!(!s.contains("provider"));
    }

    #[test]
    fn round_trip_emits_explicit_provider() {
        let r = RemoteConfig {
            url: "git@x:o/r.git".into(),
            default: false,
            provider: Some(ProviderKind::GitLab),
            push_mode: PushMode::default(),
        };
        let s = toml::to_string(&r).unwrap();
        assert!(s.contains("provider = \"gitlab\""));
    }

    #[test]
    fn remote_without_push_mode_defaults_to_pr() {
        let toml = r#"
            url = "git@example.com:o/r.git"
        "#;
        let r: RemoteConfig = ::toml::from_str(toml).unwrap();
        assert_eq!(r.push_mode, PushMode::Pr);
    }

    #[test]
    fn remote_round_trips_explicit_push_mode_direct() {
        let r = RemoteConfig {
            url: "git@example.com:o/r.git".into(),
            default: false,
            provider: None,
            push_mode: PushMode::Direct,
        };
        let s = ::toml::to_string(&r).unwrap();
        assert!(s.contains("push_mode = \"direct\""));
        let parsed: RemoteConfig = ::toml::from_str(&s).unwrap();
        assert_eq!(parsed.push_mode, PushMode::Direct);
    }

    #[test]
    fn loads_pre_meta_config() {
        let toml = r#"
            active_profile = "personal"
            [profiles.personal.user]
            email = "x@y.com"
        "#;
        let file: UserConfigFile = toml::from_str(toml).unwrap();
        assert!(!file.meta.onboarded);
        assert_eq!(file.profiles.len(), 1);
    }

    #[test]
    fn round_trip_omits_default_meta() {
        let file = UserConfigFile {
            meta: MetaSection::default(),
            ..Default::default()
        };
        let s = toml::to_string(&file).unwrap();
        assert!(!s.contains("[meta]"));
    }

    #[test]
    fn round_trip_emits_onboarded_marker() {
        let file = UserConfigFile {
            meta: MetaSection { onboarded: true },
            ..Default::default()
        };
        let s = toml::to_string(&file).unwrap();
        assert!(s.contains("onboarded = true"));
    }
}
