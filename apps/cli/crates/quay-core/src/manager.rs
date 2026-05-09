use crate::config::Config;
use crate::error::{QuayError, Result};
use crate::fetcher::{RegistryFetcher, SkillFileFetcher};
use crate::lockfile::{LockedFile, LockedRemote, LockedSkill, Lockfile};
use crate::registry::{Registry, RegistryEntry};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One file refetched by [`SkillManager::sync`]. Returned for reporting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RefetchedFile {
    pub skill: String,
    pub file: String,
}

/// Coordinates skill install/remove against the local filesystem and lockfile.
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
    pub fn new(
        config: &'a Config,
        registry_fetcher: &'a R,
        file_fetcher: &'a F,
        project_root: PathBuf,
    ) -> Self {
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

    fn lockfile_path(&self) -> PathBuf {
        self.project_root.join(".agents/skills.lock.json")
    }

    /// Resolve a skill name across all configured remotes (or one if `pinned_remote` is given).
    /// Returns the matching remote name + entry, or an error on collision/no-match.
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

    pub fn add(&self, skill_name: &str, pinned_remote: Option<&str>) -> Result<LockedSkill> {
        let (remote_name, _registry, entry) = self.resolve(skill_name, pinned_remote)?;
        let hub_url = self.config.remotes[&remote_name].url.clone();

        // Local layout is flat: one folder per skill keyed by skill name. Hub-side path
        // (entry.path) tells us where to fetch from on the hub, but the local install
        // location is always `<canonical>/<skill_name>/`.
        let dest_dir = self.install_dir().join(skill_name);
        std::fs::create_dir_all(&dest_dir).map_err(|source| QuayError::Io {
            path: dest_dir.display().to_string(),
            source,
        })?;

        let mut locked_files = Vec::new();
        // NOTE: We compute and record the sha256 of fetched bytes in the lockfile so a
        // future `quay sync` (Plan 2) can detect tampered lockfile state. We do NOT
        // currently verify the fetched content against `entry.sha` because `entry.sha`
        // is the git commit sha of the skill in the hub, not the file content sha. Once
        // Plan 7 introduces clone-based fetching with per-file commit shas (or a
        // file-content-sha field is added to registry.json), we can use
        // QuayError::IntegrityFailure here. Until then, content integrity relies on
        // HTTPS transport security.
        for file_rel in &entry.files {
            let remote_path = format!("{}/{}", entry.path, file_rel);
            let bytes = self.file_fetcher.fetch_file(&hub_url, &remote_path)?;
            let sha = sha256_hex(&bytes);
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
            locked_files.push(LockedFile {
                path: file_rel.clone(),
                sha256: sha,
            });
        }

        // Update lockfile
        let mut lock = Lockfile::load_or_default(&self.lockfile_path())?;
        lock.remotes.insert(
            remote_name.clone(),
            LockedRemote {
                url: hub_url,
                registry_sha: entry.sha.clone(),
            },
        );
        let locked = LockedSkill {
            remote: remote_name,
            version: entry.version.clone(),
            sha: entry.sha.clone(),
            path: entry.path.clone(),
            files: locked_files,
            installed_at: now_iso8601(),
        };
        lock.skills.insert(skill_name.to_string(), locked.clone());
        lock.save(&self.lockfile_path())?;

        Ok(locked)
    }

    pub fn list(&self) -> Result<Lockfile> {
        Lockfile::load_or_default(&self.lockfile_path())
    }

    pub fn remove(&self, skill_name: &str) -> Result<()> {
        let mut lock = Lockfile::load_or_default(&self.lockfile_path())?;
        if lock.skills.remove(skill_name).is_none() {
            return Err(QuayError::SkillNotFound {
                name: skill_name.into(),
                remote: "local".into(),
            });
        }
        let skill_dir = self.install_dir().join(skill_name);
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir).map_err(|source| QuayError::Io {
                path: skill_dir.display().to_string(),
                source,
            })?;
        }
        lock.save(&self.lockfile_path())?;
        Ok(())
    }

    pub fn info(&self, skill_name: &str, pinned_remote: Option<&str>) -> Result<RegistryEntry> {
        let (_, _, entry) = self.resolve(skill_name, pinned_remote)?;
        Ok(entry)
    }

    /// Update a single installed skill to the registry's current version.
    /// Returns the updated [`LockedSkill`] on upgrade, or `None` if already up to date.
    /// Errors if the skill is not installed.
    pub fn update_one(&self, skill_name: &str) -> Result<Option<LockedSkill>> {
        let lock = Lockfile::load_or_default(&self.lockfile_path())?;
        let locked = lock
            .skills
            .get(skill_name)
            .ok_or_else(|| QuayError::SkillNotFound {
                name: skill_name.into(),
                remote: "local".into(),
            })?;
        let pinned = locked.remote.clone();
        let (remote_name, _registry, entry) = self.resolve(skill_name, Some(&pinned))?;
        let installed_version = locked.version.clone();
        let available_version = entry.version.clone();
        let needs_upgrade = match (
            semver::Version::parse(&available_version),
            semver::Version::parse(&installed_version),
        ) {
            (Ok(av), Ok(in_)) => av > in_,
            _ => false,
        };
        if !needs_upgrade {
            return Ok(None);
        }
        // The existing add() routine handles fetch + write + lockfile bump.
        let updated = self.add(skill_name, Some(&remote_name))?;
        Ok(Some(updated))
    }

    /// Reproduce the lockfile exactly: for every recorded skill, ensure the canonical
    /// install path contains the recorded files with matching sha256. Refetches files
    /// pinned to the recorded commit SHA when content is missing or has drifted.
    /// Errors with [`QuayError::IntegrityFailure`] if the *fetched* bytes do not match
    /// the lockfile's recorded `sha256` for any file (suggests hub tamper).
    pub fn sync(&self) -> Result<Vec<RefetchedFile>> {
        let lock = Lockfile::load_or_default(&self.lockfile_path())?;
        let mut refetched = Vec::new();
        for (skill_name, locked) in &lock.skills {
            let Some(remote_cfg) = self.config.remotes.get(&locked.remote) else {
                return Err(QuayError::RemoteUnknown(locked.remote.clone()));
            };
            let dest_dir = self.install_dir().join(skill_name);
            std::fs::create_dir_all(&dest_dir).map_err(|source| QuayError::Io {
                path: dest_dir.display().to_string(),
                source,
            })?;
            for locked_file in &locked.files {
                let local = dest_dir.join(&locked_file.path);
                let needs_refetch = match std::fs::read(&local) {
                    Ok(bytes) => sha256_hex(&bytes) != locked_file.sha256,
                    Err(_) => true,
                };
                if !needs_refetch {
                    continue;
                }
                // Use the hub-side path recorded at install time. Falls back to
                // `skills/<skill_name>` for lockfiles produced before LockedSkill.path
                // existed (the field is `#[serde(default)]` to an empty string).
                let hub_skill_path = if locked.path.is_empty() {
                    format!("skills/{}", skill_name)
                } else {
                    locked.path.clone()
                };
                let remote_path = format!("{}/{}", hub_skill_path, locked_file.path);
                let bytes =
                    self.file_fetcher
                        .fetch_file_at(&remote_cfg.url, &remote_path, &locked.sha)?;
                let computed = sha256_hex(&bytes);
                if computed != locked_file.sha256 {
                    return Err(QuayError::IntegrityFailure {
                        path: local.display().to_string(),
                        expected: locked_file.sha256.clone(),
                        actual: computed,
                    });
                }
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
                refetched.push(RefetchedFile {
                    skill: skill_name.clone(),
                    file: locked_file.path.clone(),
                });
            }
        }
        Ok(refetched)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct FakeRegistry(Registry);
    impl RegistryFetcher for FakeRegistry {
        fn fetch(&self, _url: &str) -> Result<Registry> {
            Ok(self.0.clone())
        }
    }

    struct FakeFiles {
        files: RefCell<HashMap<String, Vec<u8>>>,
    }
    impl SkillFileFetcher for FakeFiles {
        fn fetch_file(&self, _url: &str, path: &str) -> Result<Vec<u8>> {
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

    #[test]
    fn install_writes_files_and_updates_lockfile() {
        let dir = assert_fs::TempDir::new().unwrap();
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

        let reg = Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: Some("backend".into()),
                    tags: vec!["data".into(), "backend".into()],
                    path: "skills/csv-parse".into(),
                    sha: "deadbeef".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };

        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        let locked = mgr.add("csv-parse", None).unwrap();
        assert_eq!(locked.version, "1.0.0");
        assert_eq!(locked.files.len(), 1);

        let installed = dir.path().join(".agents/skills/csv-parse/SKILL.md");
        assert!(installed.exists());
        let lock_path = dir.path().join(".agents/skills.lock.json");
        let lock_text = std::fs::read_to_string(&lock_path).unwrap();
        assert!(lock_text.contains("csv-parse"));
    }

    #[test]
    fn list_returns_empty_when_nothing_installed() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = Config::default();
        let reg = Registry {
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
        let lock = mgr.list().unwrap();
        assert!(lock.skills.is_empty());
    }

    #[test]
    fn remove_deletes_files_and_lockfile_entry() {
        let dir = assert_fs::TempDir::new().unwrap();
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
        let reg = Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                RegistryEntry {
                    version: "1.0.0".into(),
                    description: "x.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "deadbeef".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([("skills/csv-parse/SKILL.md".into(), body)])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        mgr.add("csv-parse", None).unwrap();
        mgr.remove("csv-parse").unwrap();

        let installed = dir.path().join(".agents/skills/csv-parse/SKILL.md");
        assert!(!installed.exists());
        let lock = mgr.list().unwrap();
        assert!(!lock.skills.contains_key("csv-parse"));
    }

    #[test]
    fn update_one_upgrades_when_registry_is_newer() {
        let dir = assert_fs::TempDir::new().unwrap();
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

        // First install at v1.0.0
        let reg_old = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "old-sha".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let body_old =
            b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nold body\n".to_vec();
        let files_old = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body_old.clone(),
            )])),
        };
        let regf_old = FakeRegistry(reg_old);
        let mgr = SkillManager::new(&cfg, &regf_old, &files_old, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Now swap in a registry serving v1.2.0
        let reg_new = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.2.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "new-sha".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let body_new =
            b"---\nname: csv-parse\ndescription: x.\nversion: 1.2.0\n---\nnew body\n".to_vec();
        let files_new = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body_new.clone(),
            )])),
        };
        let regf_new = FakeRegistry(reg_new);
        let mgr = SkillManager::new(&cfg, &regf_new, &files_new, dir.path().to_path_buf());

        let result = mgr.update_one("csv-parse").unwrap();
        let updated = result.expect("upgrade should occur");
        assert_eq!(updated.version, "1.2.0");
        assert_eq!(updated.sha, "new-sha");

        let on_disk = std::fs::read(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert_eq!(on_disk, body_new);
    }

    #[test]
    fn update_one_noop_when_already_current() {
        let dir = assert_fs::TempDir::new().unwrap();
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
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "x".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body,
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();
        let result = mgr.update_one("csv-parse").unwrap();
        assert!(result.is_none(), "no upgrade expected");
    }

    #[test]
    fn update_one_errors_when_not_installed() {
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
            files: std::cell::RefCell::new(std::collections::HashMap::new()),
        };
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        let err = mgr.update_one("does-not-exist").unwrap_err();
        assert!(matches!(err, QuayError::SkillNotFound { .. }));
    }

    #[test]
    fn sync_restores_missing_file() {
        let dir = assert_fs::TempDir::new().unwrap();
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

        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "abc123".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        mgr.add("csv-parse", None).unwrap();

        // Delete the installed file. sync should re-fetch it.
        std::fs::remove_file(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert!(!dir
            .path()
            .join(".agents/skills/csv-parse/SKILL.md")
            .exists());

        mgr.sync().unwrap();
        let restored = std::fs::read(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert_eq!(restored, body);
    }

    #[test]
    fn sync_errors_on_tamper() {
        // The fake fetcher will return DIFFERENT bytes than what was originally installed.
        let dir = assert_fs::TempDir::new().unwrap();
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

        let body_install =
            b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\noriginal\n".to_vec();
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "abc123".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let install_files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body_install.clone(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &install_files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Now swap to a fetcher that returns DIFFERENT bytes, simulating a tampered hub.
        let body_tampered = b"this is not the original content".to_vec();
        let tampered_files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body_tampered,
            )])),
        };
        let mgr2 = SkillManager::new(&cfg, &regf, &tampered_files, dir.path().to_path_buf());

        // Delete the local file so sync is forced to refetch.
        std::fs::remove_file(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        let err = mgr2.sync().unwrap_err();
        assert!(matches!(err, QuayError::IntegrityFailure { .. }));
    }

    #[test]
    fn sync_no_op_when_files_match_sha() {
        let dir = assert_fs::TempDir::new().unwrap();
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

        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "abc123".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Run sync — should be a no-op since the file already matches.
        mgr.sync().unwrap();
        let body_after =
            std::fs::read(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert_eq!(body_after, body);
    }

    #[test]
    fn sync_errors_when_remote_was_removed() {
        let dir = assert_fs::TempDir::new().unwrap();
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

        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "abc123".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        let files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body,
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Now remove the remote and force a refetch.
        cfg.remotes.clear();
        std::fs::remove_file(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        let mgr2 = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        let err = mgr2.sync().unwrap_err();
        assert!(matches!(err, QuayError::RemoteUnknown(_)));
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
        // Same skill name appears in both registries served by the fake.
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([("csv-parse".into(), entry.clone())]),
        };
        let regf = FakeRegistry(reg);
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body,
            )])),
        };
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        // Without --remote, this is ambiguous.
        let err = mgr.add("csv-parse", None).unwrap_err();
        assert!(matches!(err, QuayError::NameCollision { .. }));
        if let QuayError::NameCollision { remotes, .. } = err {
            assert_eq!(remotes.len(), 2);
            assert!(remotes.contains(&"alpha".to_string()));
            assert!(remotes.contains(&"beta".to_string()));
        }

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
            files: std::cell::RefCell::new(std::collections::HashMap::new()),
        };
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        let err = mgr.info("csv-parse", Some("does-not-exist")).unwrap_err();
        assert!(matches!(err, QuayError::RemoteUnknown(_)));
    }

    #[test]
    fn sync_works_for_nested_hub_layout() {
        let dir = assert_fs::TempDir::new().unwrap();
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

        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        // Hub places this skill under skills/backend/csv-parse (nested category folder).
        let reg = crate::registry::Registry {
            hub: "fixture".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: std::collections::BTreeMap::from([(
                "csv-parse".into(),
                crate::registry::RegistryEntry {
                    version: "1.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: Some("backend".into()),
                    tags: vec![],
                    path: "skills/backend/csv-parse".into(),
                    sha: "abc123".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: crate::scanner::SkillFormat::Frontmatter,
                },
            )]),
        };
        // The fake serves the file at the nested path.
        let files = FakeFiles {
            files: std::cell::RefCell::new(std::collections::HashMap::from([(
                "skills/backend/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        mgr.add("csv-parse", None).unwrap();

        // Delete the local file and run sync. It must request the nested path,
        // not skills/csv-parse/SKILL.md, or the FakeFiles lookup will miss.
        std::fs::remove_file(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        mgr.sync().unwrap();
        let restored = std::fs::read(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert_eq!(restored, body);
    }
}
