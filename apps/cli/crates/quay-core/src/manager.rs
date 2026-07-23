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
    pub fn resolve(
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
            let remote_cfg = &self.config.remotes[&remote_name];
            let url = &remote_cfg.url;
            let registry = match remote_cfg.direct_branch.as_deref() {
                Some(b) => self.registry_fetcher.fetch_at(url, b)?,
                None => self.registry_fetcher.fetch(url)?,
            };
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
                // SAFETY: match arm guarantees exactly one candidate.
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
        let remote_cfg = &self.config.remotes[&_remote_name];
        let hub_url = remote_cfg.url.clone();
        let direct_branch = remote_cfg.direct_branch.clone();

        let dest_dir = self.install_dir().join(skill_name);

        if !force && dest_dir.exists() {
            return Err(QuayError::AlreadyExists(dest_dir.display().to_string()));
        }

        let install_dir = self.install_dir();
        std::fs::create_dir_all(&install_dir).map_err(|source| QuayError::Io {
            path: install_dir.display().to_string(),
            source,
        })?;

        // Fetch into a staging dir, then rename it into place. A fetch that returns
        // early via `?` leaves nothing behind: TempDir removes itself on drop.
        // Writing directly into dest_dir would strand a partial skill that then
        // blocks its own retry with AlreadyExists.
        //
        // Drop does not run on SIGINT/SIGKILL, so a killed fetch still strands a
        // staging dir — which is the other reason it does not live in the install
        // dir (see below).
        //
        // Staging lives under `.quay/`, not in the install dir: it is inside the
        // project (so the rename stays on one filesystem, and therefore atomic)
        // but outside every root `scanner::scan_project` walks. A staging dir
        // orphaned by SIGKILL is invisible rather than showing up in `quay list`
        // as a skill named `.tmpAbCdEf`.
        // ponytail: no reaper for those orphans — they are hidden and rare. Add a
        // sweep here if they ever accumulate, but it must not race a concurrent add.
        let staging_root = self.project_root.join(".quay");
        std::fs::create_dir_all(&staging_root).map_err(|source| QuayError::Io {
            path: staging_root.display().to_string(),
            source,
        })?;
        let staging = tempfile::TempDir::new_in(&staging_root).map_err(|source| QuayError::Io {
            path: staging_root.display().to_string(),
            source,
        })?;

        for file_rel in &entry.files {
            // registry.json comes off the network, so its file list is untrusted
            // input. `Path::join` silently discards the base when handed an
            // absolute path, and honours `..` — an entry of "../../.ssh/authorized_keys"
            // would otherwise write straight through the staging dir.
            reject_unsafe_path(file_rel)?;
            let remote_path = format!("{}/{}", entry.path, file_rel);
            let bytes = match direct_branch.as_deref() {
                Some(b) => self.file_fetcher.fetch_file_at(&hub_url, &remote_path, b)?,
                None => self.file_fetcher.fetch_file(&hub_url, &remote_path)?,
            };
            let local = staging.path().join(file_rel);
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

        // Every file landed; commit the swap.
        //
        // Re-check rather than trusting the check at the top: the fetch loop above
        // is seconds to minutes of network, and a `force = false` add must never
        // delete a directory that appeared during it.
        if dest_dir.exists() {
            if !force {
                return Err(QuayError::AlreadyExists(dest_dir.display().to_string()));
            }
            // Overwriting in place used to leave files that aren't in the manifest
            // (local notes, files dropped upstream) untouched, because it wrote
            // over the directory rather than replacing it. `quay update` runs this
            // path on every skill, so preserve them: whether `update` should clobber
            // them is an open product question, not something to settle in a patch.
            copy_missing_into(&dest_dir, staging.path())?;
        }

        // Move the old tree aside instead of deleting it, so a failed rename can
        // put it back. Deleting first would mean a failure at exactly the wrong
        // moment leaves the skill gone entirely — worse than the in-place
        // overwrite this replaces.
        let backup = dest_dir.exists().then(|| {
            let mut p = dest_dir.clone();
            p.set_file_name(format!(".{skill_name}.replaced"));
            p
        });
        if let Some(backup) = &backup {
            let _ = std::fs::remove_dir_all(backup);
            std::fs::rename(&dest_dir, backup).map_err(|source| QuayError::Io {
                path: dest_dir.display().to_string(),
                source,
            })?;
        }

        let staged = staging.keep();
        if let Err(source) = std::fs::rename(&staged, &dest_dir) {
            // Put the original back before surfacing the error, and don't strand
            // the staging copy on disk. Both are best-effort, but say so when they
            // fail: the user is about to get a rename errno, and "your skill is in
            // <path>" is the difference between recoverable and not.
            if let Some(backup) = &backup {
                if let Err(e) = std::fs::rename(backup, &dest_dir) {
                    eprintln!(
                        "warning: could not restore {} from {}: {e}; the previous copy is still there",
                        dest_dir.display(),
                        backup.display()
                    );
                }
            }
            if let Err(e) = std::fs::remove_dir_all(&staged) {
                eprintln!(
                    "warning: could not remove staging dir {}: {e}; the fetched copy is intact there",
                    staged.display()
                );
            }
            return Err(QuayError::Io {
                path: dest_dir.display().to_string(),
                source,
            });
        }
        if let Some(backup) = &backup {
            if let Err(e) = std::fs::remove_dir_all(backup) {
                eprintln!(
                    "warning: install succeeded but the replaced copy remains at {}: {e}",
                    backup.display()
                );
            }
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

/// Reject a registry-supplied file path that would escape the directory it is
/// joined onto.
///
/// Rejects absolute paths (`Path::join` throws the base away), any `..`
/// component, and Windows path prefixes such as `C:` or `\\server\share` — which
/// `Component::Prefix` catches on Windows and the explicit `\` and `:` checks
/// catch when a Windows-authored registry is consumed on Unix, where the whole
/// string would otherwise be treated as one innocent-looking filename.
fn reject_unsafe_path(file_rel: &str) -> Result<()> {
    use std::path::Component;

    let bad = |reason: &str| {
        Err(QuayError::InvalidRegistry {
            reason: format!("unsafe file path {file_rel:?} in registry entry: {reason}"),
        })
    };

    if file_rel.is_empty() {
        return bad("empty");
    }
    if file_rel.contains('\0') {
        return bad("contains a null byte");
    }
    // Checked before parsing: on Unix these are ordinary filename characters, so
    // Components would not flag them.
    if file_rel.contains('\\') {
        return bad("contains a backslash");
    }
    if file_rel.chars().nth(1) == Some(':') {
        return bad("looks like a Windows drive path");
    }

    let path = Path::new(file_rel);
    if path.is_absolute() || path.has_root() {
        return bad("is absolute");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return bad("contains a `..` component"),
            Component::RootDir | Component::Prefix(_) => return bad("is absolute"),
        }
    }
    Ok(())
}

/// Recursively copy anything under `from` that `into` does not already have.
///
/// Used to carry files the manifest doesn't list — local notes, files dropped
/// upstream — across a replace, matching what writing over the directory in
/// place used to do. Files present in `into` win: those are the freshly fetched
/// ones.
fn copy_missing_into(from: &Path, into: &Path) -> Result<()> {
    let entries = std::fs::read_dir(from).map_err(|source| QuayError::Io {
        path: from.display().to_string(),
        source,
    })?;
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = into.join(entry.file_name());
        if src.is_dir() {
            std::fs::create_dir_all(&dst).map_err(|source| QuayError::Io {
                path: dst.display().to_string(),
                source,
            })?;
            copy_missing_into(&src, &dst)?;
        } else if !dst.exists() {
            std::fs::copy(&src, &dst).map_err(|source| QuayError::Io {
                path: dst.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
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
                    content_hash: String::new(),
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
                direct_branch: None,
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

    /// A fetch that dies partway must leave nothing behind — no partial skill
    /// dir (which would block the retry with AlreadyExists) and no staging dir.
    #[test]
    fn add_leaves_nothing_behind_when_a_fetch_fails_midway() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();

        // Two files, only the first fetchable: the loop dies on the second.
        let mut reg = make_registry("csv-parse", "1.0.0");
        reg.skills.get_mut("csv-parse").unwrap().files =
            vec!["SKILL.md".into(), "reference.md".into()];
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        let err = mgr.add("csv-parse", None).unwrap_err();
        assert!(
            matches!(err, QuayError::SkillNotFound { .. }),
            "got {err:?}"
        );

        let dest = dir.path().join(".agents/skills/csv-parse");
        assert!(
            !dest.exists(),
            "partial skill dir must not survive a failed fetch"
        );

        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join(".agents/skills"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging dir must be cleaned up, found {leftovers:?}"
        );

        // The retry is the point: without staging this hit AlreadyExists.
        files.files.borrow_mut().insert(
            "skills/csv-parse/reference.md".into(),
            b"reference\n".to_vec(),
        );
        mgr.add("csv-parse", None).unwrap();
        assert!(dest.join("SKILL.md").exists());
        assert!(dest.join("reference.md").exists());
    }

    /// `quay update` runs the force path on every skill. A fetch that dies partway
    /// through it must leave the working install exactly as it was — the ordering
    /// that guarantees this (swap only after every file has landed) is easy to
    /// "simplify" away, and the cost of getting it wrong is a deleted skill.
    #[test]
    fn force_add_leaves_the_existing_install_intact_when_a_fetch_fails() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let original = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\noriginal\n";
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                original.to_vec(),
            )])),
        };
        let regf = FakeRegistry(make_registry("csv-parse", "1.0.0"));
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Second version adds a file that cannot be fetched.
        let mut reg2 = make_registry("csv-parse", "2.0.0");
        reg2.skills.get_mut("csv-parse").unwrap().files =
            vec!["SKILL.md".into(), "reference.md".into()];
        files.files.borrow_mut().insert(
            "skills/csv-parse/SKILL.md".into(),
            b"---\nname: csv-parse\ndescription: x.\nversion: 2.0.0\n---\nnew\n".to_vec(),
        );
        let regf2 = FakeRegistry(reg2);
        let mgr2 = SkillManager::new(&cfg, &regf2, &files, dir.path().to_path_buf());

        mgr2.add_with_force("csv-parse", None, true).unwrap_err();

        let installed =
            std::fs::read(dir.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
        assert_eq!(
            installed, original,
            "a failed force-add must not disturb the working install"
        );
        let names: Vec<_> = std::fs::read_dir(dir.path().join(".agents/skills"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            names.len(),
            1,
            "no backup or staging left behind: {names:?}"
        );
    }

    /// Overwriting used to write over the directory, so files outside the manifest
    /// survived. Replacing it wholesale would silently delete them on every
    /// `quay update` — whether it should is an open question, not a patch-level
    /// decision.
    #[test]
    fn force_add_keeps_files_outside_the_manifest() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(make_registry("csv-parse", "1.0.0"));
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join("NOTES.md"), b"my notes").unwrap();
        std::fs::create_dir_all(skill_dir.join("local")).unwrap();
        std::fs::write(skill_dir.join("local/scratch.txt"), b"scratch").unwrap();

        mgr.add_with_force("csv-parse", None, true).unwrap();

        assert_eq!(
            std::fs::read(skill_dir.join("NOTES.md")).unwrap(),
            b"my notes",
            "unmanifested file must survive a force overwrite"
        );
        assert_eq!(
            std::fs::read(skill_dir.join("local/scratch.txt")).unwrap(),
            b"scratch",
            "nested unmanifested file must survive too"
        );
        assert!(skill_dir.join("SKILL.md").exists());
    }

    /// The rename is only atomic while staging and destination share a filesystem.
    /// Staging under `/tmp` would pass on a dev box and fail on any user whose
    /// `/tmp` is a tmpfs, so pin where it lives.
    #[test]
    fn staging_stays_inside_the_project_and_out_of_the_scanned_dir() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/SKILL.md".into(),
                body.clone(),
            )])),
        };
        let regf = FakeRegistry(make_registry("csv-parse", "1.0.0"));
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add("csv-parse", None).unwrap();

        // Success path must not leave staging behind either.
        let names: Vec<_> = std::fs::read_dir(dir.path().join(".agents/skills"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["csv-parse"],
            "install dir must hold only the skill, no staging residue"
        );
        // .quay is inside the project (same filesystem as the install dir) and is
        // not one of the roots the scanner walks.
        assert!(dir.path().join(".quay").exists(), "staging root is .quay/");
    }

    #[test]
    fn unsafe_registry_paths_are_rejected() {
        for bad in [
            "../evil.md",
            "a/../../evil.md",
            "/etc/passwd",
            "//server/share/x",
            "C:/Windows/system32/x",
            r"..\evil.md",
            r"dir\file.md",
            "",
        ] {
            assert!(
                reject_unsafe_path(bad).is_err(),
                "should have rejected {bad:?}"
            );
        }
        for ok in ["SKILL.md", "scripts/run.py", "./SKILL.md", "a/b/c.md"] {
            assert!(reject_unsafe_path(ok).is_ok(), "should have allowed {ok:?}");
        }
    }

    /// A hostile or compromised hub controls registry.json. Nothing may be
    /// written outside the skill's own directory.
    #[test]
    fn add_refuses_a_registry_entry_that_escapes_the_skill_dir() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let mut reg = make_registry("csv-parse", "1.0.0");
        reg.skills.get_mut("csv-parse").unwrap().files = vec!["../../pwned.md".into()];
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([(
                "skills/csv-parse/../../pwned.md".into(),
                b"pwned".to_vec(),
            )])),
        };
        let regf = FakeRegistry(reg);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());

        let err = mgr.add("csv-parse", None).unwrap_err();
        assert!(
            matches!(err, QuayError::InvalidRegistry { .. }),
            "got {err:?}"
        );
        assert!(
            !dir.path().join("pwned.md").exists(),
            "traversal must not write outside the skill dir"
        );
        assert!(!dir.path().join(".agents/skills/csv-parse").exists());
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
                direct_branch: None,
            },
        );
        cfg.remotes.insert(
            "beta".into(),
            crate::config::RemoteConfig {
                url: "https://github.com/p/q.git".into(),
                default: false,
                provider: None,
                push_mode: crate::config::PushMode::default(),
                direct_branch: None,
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
            content_hash: String::new(),
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
