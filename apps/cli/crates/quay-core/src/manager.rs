//! Skill install / remove coordinator — filesystem-first, no lockfile.
//!
//! As of quay 0.2.0, skills are tracked by git history and the filesystem.
//! There is no `skills.lock.json`. If a legacy lockfile is detected at startup
//! a one-time notice is printed to stderr.

use crate::config::{Config, MirrorRoot};
use crate::error::{QuayError, Result};
use crate::fetcher::{RegistryFetcher, SkillFileFetcher};
use crate::registry::{Registry, RegistryEntry};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Coordinates skill install / remove against the local filesystem.
///
/// There is no lockfile in 0.2.0. Skills are found by scanning the
/// filesystem; reproducibility is delegated to git history.
pub struct SkillManager<'a, R, F>
where
    R: RegistryFetcher,
    F: SkillFileFetcher,
{
    pub config: &'a Config,
    pub registry_fetcher: &'a R,
    pub file_fetcher: &'a F,
    pub project_root: PathBuf,
}

impl<'a, R, F> SkillManager<'a, R, F>
where
    R: RegistryFetcher,
    F: SkillFileFetcher,
{
    /// Create a new manager. Also prints a one-time migration notice if
    /// `skills.lock.json` is found.
    pub fn new(
        config: &'a Config,
        registry_fetcher: &'a R,
        file_fetcher: &'a F,
        project_root: PathBuf,
    ) -> Self {
        check_legacy_lockfile(&project_root);
        Self {
            config,
            registry_fetcher,
            file_fetcher,
            project_root,
        }
    }

    fn install_dir(&self) -> PathBuf {
        self.project_root.join(&self.config.install.canonical)
    }

    /// Resolve a skill name across all configured remotes (or one if `pinned_remote` is given).
    fn resolve(
        &self,
        skill_name: &str,
        pinned_remote: Option<&str>,
    ) -> Result<(String, Registry, RegistryEntry)> {
        let candidates: Vec<String> = match pinned_remote {
            Some(name) => {
                if !self.config.remotes.contains_key(name) {
                    return Err(QuayError::RemoteUnknown(name.into()));
                }
                vec![name.to_string()]
            }
            None => self.config.remotes.keys().cloned().collect(),
        };
        let mut matches = BTreeMap::new();
        for remote_name in candidates {
            let url = &self.config.remotes[&remote_name].url;
            let registry = self.registry_fetcher.fetch(url)?;
            if let Some(entry) = registry.entry(skill_name) {
                matches.insert(remote_name, (registry.clone(), entry.clone()));
            }
        }
        match matches.len() {
            0 => Err(QuayError::SkillNotFound {
                name: skill_name.into(),
                remote: pinned_remote.unwrap_or("any").into(),
            }),
            1 => {
                let (remote, (reg, entry)) = matches.into_iter().next().unwrap();
                Ok((remote, reg, entry))
            }
            _ => Err(QuayError::NameCollision {
                name: skill_name.into(),
                remotes: matches.keys().cloned().collect(),
            }),
        }
    }

    /// Fetch a skill from a remote and write it to the canonical install directory.
    ///
    /// If `force` is `false` and the skill directory already exists, returns
    /// [`QuayError::AlreadyExists`].  Pass `force = true` to overwrite.
    pub fn add(&self, skill_name: &str, pinned_remote: Option<&str>) -> Result<()> {
        self.add_with_force(skill_name, pinned_remote, false)
    }

    /// Like [`add`] but with explicit overwrite control.
    pub fn add_with_force(
        &self,
        skill_name: &str,
        pinned_remote: Option<&str>,
        force: bool,
    ) -> Result<()> {
        let (_remote_name, _registry, entry) = self.resolve(skill_name, pinned_remote)?;
        let hub_url = self.config.remotes[&_remote_name].url.clone();

        let dest_dir = self.install_dir().join(skill_name);

        if !force && dest_dir.exists() {
            return Err(QuayError::AlreadyExists(dest_dir.display().to_string()));
        }

        std::fs::create_dir_all(&dest_dir).map_err(|source| QuayError::Io {
            path: dest_dir.display().to_string(),
            source,
        })?;

        for file_rel in &entry.files {
            let remote_path = format!("{}/{}", entry.path, file_rel);
            let bytes = self.file_fetcher.fetch_file(&hub_url, &remote_path)?;
            let local = dest_dir.join(file_rel);
            if let Some(parent) = local.parent() {
                std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
            std::fs::write(&local, &bytes).map_err(|source| QuayError::Io {
                path: local.display().to_string(),
                source,
            })?;
        }

        Ok(())
    }

    /// Remove a skill from all local mirror roots.
    ///
    /// Removes the skill directory from every [`MirrorRoot`] that contains it.
    /// Does not interact with any remote.
    pub fn remove(&self, skill_name: &str) -> Result<()> {
        let mut removed_any = false;
        for mirror in MirrorRoot::all() {
            let skill_dir = self.project_root.join(mirror.dir()).join(skill_name);
            if skill_dir.exists() {
                std::fs::remove_dir_all(&skill_dir).map_err(|source| QuayError::Io {
                    path: skill_dir.display().to_string(),
                    source,
                })?;
                removed_any = true;
            }
        }
        if !removed_any {
            return Err(QuayError::SkillNotFound {
                name: skill_name.into(),
                remote: "local".into(),
            });
        }
        Ok(())
    }

    /// Show registry metadata for a skill without installing it.
    pub fn info(&self, skill_name: &str, pinned_remote: Option<&str>) -> Result<RegistryEntry> {
        let (_, _, entry) = self.resolve(skill_name, pinned_remote)?;
        Ok(entry)
    }

    /// Update a skill to the latest available version.
    ///
    /// Re-fetches and overwrites the local file(s). The old content is
    /// captured in git history by the user's normal git workflow.
    pub fn update_one(&self, skill_name: &str) -> Result<bool> {
        // Force-overwrite is always fine on update.
        self.add_with_force(skill_name, None, true)?;
        Ok(true)
    }
}

/// Print a one-time migration notice if legacy state files are present.
///
/// Does not block or abort.
fn check_legacy_lockfile(project_root: &Path) {
    let lockfile = project_root.join(".agents").join("skills.lock.json");
    if lockfile.exists() {
        eprintln!("note: `skills.lock.json` is no longer used as of quay 0.2.0.");
        eprintln!("      delete it: rm {}", lockfile.display());
        eprintln!("      installed skills are tracked by your repo's git history.");
    }

    let push_log = project_root.join(".quay").join("push-log.json");
    if push_log.exists() {
        eprintln!("note: per-project .quay/push-log.json is no longer used as of quay 0.2.x.");
        eprintln!(
            "      its contents have been migrated into ~/.config/quay/push-log.json on first push;"
        );
        eprintln!(
            "      you can delete the local file: rm {}",
            push_log.display()
        );
    }
}

/// Compute the SHA-256 hex digest of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct FakeRegistry(Registry);
    impl RegistryFetcher for FakeRegistry {
        fn fetch(&self, _url: &str) -> crate::error::Result<Registry> {
            Ok(self.0.clone())
        }
    }

    struct FakeFiles {
        files: RefCell<HashMap<String, Vec<u8>>>,
    }
    impl SkillFileFetcher for FakeFiles {
        fn fetch_file(&self, _url: &str, path: &str) -> crate::error::Result<Vec<u8>> {
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| QuayError::SkillNotFound {
                    name: path.into(),
                    remote: "fake".into(),
                })
        }
    }

    fn make_registry(skill_name: &str, version: &str) -> Registry {
        Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-10T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                skill_name.to_string(),
                crate::registry::RegistryEntry {
                    version: version.into(),
                    description: "test skill".into(),
                    category: None,
                    tags: vec![],
                    path: format!("skills/{}", skill_name),
                    sha: "abc123".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        }
    }

    fn make_cfg() -> Config {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            crate::config::RemoteConfig {
                url: "https://github.com/foo/bar.git".into(),
                default: true,
                provider: None,
                push_mode: crate::config::PushMode::default(),
            },
        );
        cfg
    }

    #[test]
    fn add_writes_skill_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let reg = make_registry("csv-parse", "1.0.0");
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        mgr.add("csv-parse", None).unwrap();

        let installed = dir.path().join(".agents/skills/csv-parse/SKILL.md");
        assert!(installed.exists(), "file should be written");
        // No lockfile must be created.
        assert!(
            !dir.path().join(".agents/skills.lock.json").exists(),
            "lockfile must NOT be created"
        );
    }

    #[test]
    fn add_errors_when_already_exists_without_force() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let reg = make_registry("csv-parse", "1.0.0");
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        mgr.add("csv-parse", None).unwrap();
        let err = mgr.add("csv-parse", None).unwrap_err();
        assert!(matches!(err, QuayError::AlreadyExists(_)));
    }

    #[test]
    fn add_with_force_overwrites() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body1 =
            b"---\nname: csv-parse\ndescription: v1.\nversion: 1.0.0\n---\nbody1\n".to_vec();
        let body2 =
            b"---\nname: csv-parse\ndescription: v2.\nversion: 2.0.0\n---\nbody2\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body1.clone(),
            )])),
        };
        let reg = make_registry("csv-parse", "1.0.0");
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Now swap to body2 and force-overwrite.
        let files2 = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body2.clone(),
            )])),
        };
        let reg2 = make_registry("csv-parse", "2.0.0");
        let regf2 = FakeRegistry(reg2);
        let mgr2 = SkillManager::new(&cfg, &regf2, &files2, dir.path().to_path_buf());
        mgr2.add_with_force("csv-parse", None, true).unwrap();

        let on_disk = std::fs::read(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert_eq!(on_disk, body2);
    }

    #[test]
    fn remove_deletes_from_agents_root() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let reg = make_registry("csv-parse", "1.0.0");
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        let installed = dir.path().join(".agents/skills/csv-parse/SKILL.md");
        assert!(installed.exists());

        mgr.remove("csv-parse").unwrap();
        assert!(!installed.exists(), "file must be removed");
    }

    #[test]
    fn remove_also_deletes_from_other_mirrors() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let reg = make_registry("csv-parse", "1.0.0");
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Manually create a mirror copy.
        let claude_dir = dir.path().join(".claude/skills/csv-parse");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("SKILL.md"), &body).unwrap();

        mgr.remove("csv-parse").unwrap();

        assert!(!dir.path().join(".agents/skills/csv-parse").exists());
        assert!(!dir.path().join(".claude/skills/csv-parse").exists());
    }

    #[test]
    fn remove_errors_when_skill_not_installed() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = Config::default();
        let reg = make_registry("none", "0.0.0");
        let regf = FakeRegistry(reg);
        let files = FakeFiles {
            files: RefCell::new(HashMap::new()),
        };
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        let err = mgr.remove("does-not-exist").unwrap_err();
        assert!(matches!(err, QuayError::SkillNotFound { .. }));
    }

    #[test]
    fn add_errors_on_skill_name_collision_across_remotes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "alpha".into(),
            crate::config::RemoteConfig {
                url: "https://github.com/x/y.git".into(),
                default: false,
                provider: None,
                push_mode: crate::config::PushMode::default(),
            },
        );
        cfg.remotes.insert(
            "beta".into(),
            crate::config::RemoteConfig {
                url: "https://github.com/p/q.git".into(),
                default: false,
                provider: None,
                push_mode: crate::config::PushMode::default(),
            },
        );
        let entry = crate::registry::RegistryEntry {
            version: "1.0.0".into(),
            description: "x.".into(),
            category: None,
            tags: vec![],
            path: "skills/csv-parse".into(),
            sha: "abc".into(),
            files: vec!["SKILL.md".into()],
            source_format: crate::scanner::SkillFormat::Frontmatter,
        };
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([("csv-parse".into(), entry.clone())]),
        };
        let regf = FakeRegistry(reg);
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([("skills/csv-parse/SKILL.md".into(), body)])),
        };
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        let err = mgr.add("csv-parse", None).unwrap_err();
        assert!(matches!(err, QuayError::NameCollision { .. }));

        // With --remote pinned, it succeeds.
        mgr.add("csv-parse", Some("alpha")).unwrap();
    }

    #[test]
    fn info_with_unknown_remote_errors() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = Config::default();
        let reg = crate::registry::Registry {
            hub: "x".into(),
            generated_at: "x".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::new(),
        };
        let regf = FakeRegistry(reg);
        let files = FakeFiles {
            files: RefCell::new(HashMap::new()),
        };
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        let err = mgr.info("csv-parse", Some("does-not-exist")).unwrap_err();
        assert!(matches!(err, QuayError::RemoteUnknown(_)));
    }
}
