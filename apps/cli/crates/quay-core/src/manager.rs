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
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What to do with local files the new version does not contain.
///
/// Paths are skill-relative with POSIX separators, matching what
/// [`crate::skill_files::collect_skill_files`] produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraFiles {
    /// Carry every extra file forward — the historical behaviour.
    Keep,
    /// Drop every extra file.
    Delete,
    /// Drop exactly these, keep the rest. Paths not in the offered set are
    /// ignored rather than trusted.
    DeleteOnly(Vec<String>),
}

/// Decides what happens to extra files, called once per skill and only when the
/// extra set is non-empty.
///
/// Returning `Err` aborts the install **before** the swap, leaving the existing
/// copy untouched — that is how an interrupted prompt cancels an update rather
/// than silently taking a default.
pub type DecideExtras<'a> = &'a dyn Fn(&str, &[String]) -> Result<ExtraFiles>;

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
    ///
    /// Keeps every local file the new version does not contain — the historical
    /// behaviour, and what non-interactive callers such as `quay-mcp` want.
    /// Nothing local is dropped: a directory that was already empty is
    /// recreated too. Use [`add_with_extras`] to let the caller decide instead.
    pub fn add_with_force(
        &self,
        skill_name: &str,
        pinned_remote: Option<&str>,
        force: bool,
    ) -> Result<()> {
        self.add_with_extras(skill_name, pinned_remote, force, &|_, _| {
            Ok(ExtraFiles::Keep)
        })
    }

    /// Like [`add_with_force`], but `decide` chooses what happens to local files
    /// the fetched version does not contain.
    ///
    /// `decide` is called once, only when that set is non-empty, and only on the
    /// `force` path — a non-force add errors on a pre-existing directory before
    /// reaching it. Returning `Err` aborts before anything on disk changes.
    pub fn add_with_extras(
        &self,
        skill_name: &str,
        pinned_remote: Option<&str>,
        force: bool,
        decide: DecideExtras<'_>,
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
            // Files the old install has and the new version does not. Historically
            // all of them were carried forward, which preserved hand-added notes
            // and also resurrected every file the hub had deleted. `decide` is
            // where that is settled — see
            // docs/superpowers/specs/2026-07-31-update-extra-files-design.md.
            let extras = compute_extras(&dest_dir, staging.path())?;
            let skip: BTreeSet<String> = if extras.is_empty() {
                BTreeSet::new()
            } else {
                match decide(skill_name, &extras)? {
                    ExtraFiles::Keep => BTreeSet::new(),
                    ExtraFiles::Delete => extras.iter().cloned().collect(),
                    // Intersect rather than trust: `decide` lives in another
                    // crate, and a path it names that was never offered would
                    // otherwise delete a file the user was never asked about.
                    ExtraFiles::DeleteOnly(chosen) => {
                        let offered: BTreeSet<&str> = extras.iter().map(String::as_str).collect();
                        chosen
                            .into_iter()
                            .filter(|p| offered.contains(p.as_str()))
                            .collect()
                    }
                }
            };
            copy_missing_into(&dest_dir, staging.path(), &skip)?;
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
    /// Re-fetches and overwrites the local file(s), keeping every local file the
    /// new version does not contain. The old content is captured in git history
    /// by the user's normal git workflow.
    pub fn update_one(&self, skill_name: &str) -> Result<bool> {
        self.update_one_with_extras(skill_name, &|_, _| Ok(ExtraFiles::Keep))
    }

    /// Like [`update_one`], but `decide` chooses what happens to local files the
    /// new version does not contain.
    pub fn update_one_with_extras(
        &self,
        skill_name: &str,
        decide: DecideExtras<'_>,
    ) -> Result<bool> {
        // Force-overwrite is always fine on update.
        self.add_with_extras(skill_name, None, true, decide)?;
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

/// Copy files present in `from` but missing from `into`, except those named in
/// `skip`.
///
/// `into` is the staging dir that is about to be renamed over the install, so a
/// file is deleted purely by *not* being copied — no `remove_file` runs, and the
/// atomic swap keeps its crash-safety.
fn copy_missing_into(from: &Path, into: &Path, skip: &BTreeSet<String>) -> Result<()> {
    copy_missing_rec(from, into, "", skip)
}

/// `prefix` is the POSIX-joined path of `from` relative to the skill root, so
/// `skip` keys match what `collect_skill_files` produced. Comparing an `OsStr`
/// path here instead would silently stop matching on Windows.
fn copy_missing_rec(from: &Path, into: &Path, prefix: &str, skip: &BTreeSet<String>) -> Result<()> {
    let mut entries = std::fs::read_dir(from)
        .map_err(|source| QuayError::Io {
            path: from.display().to_string(),
            source,
        })?
        .peekable();
    // A directory with no entries at all has no child to materialize it
    // lazily below, so without this it would vanish on every update — deletion
    // by omission, on the path that promises to keep everything. The test is
    // "no entries at all" rather than "nothing was copied" precisely so the
    // other case still holds: a directory whose every child was skipped is a
    // husk and must not be recreated.
    //
    // Something already at `into` is the freshly fetched copy, which always
    // wins — including when it is a file of the same name.
    if entries.peek().is_none() {
        if !into.exists() {
            std::fs::create_dir_all(into).map_err(|source| QuayError::Io {
                path: into.display().to_string(),
                source,
            })?;
        }
        return Ok(());
    }
    for entry in entries {
        // Not `.flatten()`: this walk *is* the deletion mechanism, so an entry
        // dropped here is a file destroyed by omission. A transient EIO or a
        // permission change mid-walk must fail the install, not silently take
        // a file with it.
        let entry = entry.map_err(|source| QuayError::Io {
            path: from.display().to_string(),
            source,
        })?;
        let src = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let dst = into.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source| QuayError::Io {
            path: src.display().to_string(),
            source,
        })?;
        if file_type.is_symlink() {
            // Symlinks are outside `collect_skill_files`'s managed set, so
            // `compute_extras` never lists one and `skip` never names one —
            // it is always carried forward. `std::fs::copy` follows a symlink
            // and would silently replace it with a plain copy of its target's
            // bytes (or, for a symlink to a directory, `is_dir()` below would
            // recurse into the target as if it belonged to this skill), so
            // the link itself is recreated instead of dereferenced.
            if !dst.exists() {
                copy_symlink(&src, &dst, &rel)?;
            }
        } else if file_type.is_dir() {
            // Recurse without creating `dst` first: a directory with children
            // materializes only when a file actually lands in it, so skipping
            // every child leaves no empty husk behind. A childless directory is
            // created by the recursive call itself (see the top of this
            // function).
            copy_missing_rec(&src, &dst, &rel, skip)?;
        } else if !skip.contains(&rel) && !dst.exists() {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
            std::fs::copy(&src, &dst).map_err(|source| QuayError::Io {
                path: dst.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

/// Recreate the symlink `src` at `dst`, preserving the link rather than
/// dereferencing it.
///
/// `rel` exists only to name the link in the Windows degrade path's warnings
/// (see [`degrade_symlink_failure`]); recreating a symlink never consults
/// `skip`, since symlinks are outside the managed set entirely.
fn copy_symlink(src: &Path, dst: &Path, rel: &str) -> Result<()> {
    let link_target = std::fs::read_link(src).map_err(|source| QuayError::Io {
        path: src.display().to_string(),
        source,
    })?;
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    match create_symlink_at(src, &link_target, dst) {
        Ok(()) => Ok(()),
        Err(e) => degrade_symlink_failure(src, dst, &link_target, rel, e),
    }
}

/// Whether a symlink-creation failure is the class Windows raises when the
/// caller lacks Developer Mode or elevation (`ERROR_PRIVILEGE_NOT_HELD`), as
/// opposed to a real failure — a full disk, a broken object store — that must
/// still propagate.
///
/// Matches the raw code as well as the mapped kind: whether std decodes 1314
/// to `PermissionDenied` or leaves it `Uncategorized` is an implementation
/// detail, and getting it wrong means the degrade never fires and Windows
/// hard-fails on any skill containing a symlink.
///
/// Split out from [`degrade_symlink_failure`] so the classification is
/// testable on every platform; the fallback behaviour it gates is
/// Windows-only, so this is otherwise dead weight on every other target.
#[cfg(any(windows, test))]
fn is_permission_class_failure(e: &QuayError) -> bool {
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
    matches!(
        e,
        QuayError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::PermissionDenied
                || source.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD)
    )
}

/// `symlink_file`/`symlink_dir` require Developer Mode or elevation on
/// Windows, which ordinary users have neither of — one symlink anywhere in a
/// skill must not turn `update` / `add --force` into a permanent hard
/// failure. A permission-class failure on a *file* link falls back to the
/// pre-fix behaviour (copy the dereferenced target) and warns; anything else —
/// a full disk, a broken object store — is a real failure and still
/// propagates. Unix never takes this path differently: [`create_symlink_at`]
/// on unix does not fail this way in practice, and if it ever does, the
/// error still propagates exactly as it did before this function existed.
#[cfg(windows)]
fn degrade_symlink_failure(
    src: &Path,
    dst: &Path,
    link_target: &Path,
    rel: &str,
    e: QuayError,
) -> Result<()> {
    if !is_permission_class_failure(&e) {
        return Err(e);
    }
    let meta = match std::fs::metadata(src) {
        Ok(meta) => meta,
        // A dangling link resolves to nothing, so there is no content to lose
        // by dropping it. Every other errno — a denied parent, ELOOP,
        // ENAMETOOLONG, EIO — says nothing about whether the target has
        // contents, and `dst` is the staging tree about to be renamed over the
        // install, so swallowing it would destroy the entry while reporting
        // success.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "warning: could not recreate symlink {rel} -> {} (Developer Mode or elevation \
                 required), and it dangles, so there are no contents to copy; dropping it",
                link_target.display()
            );
            return Ok(());
        }
        Err(source) => {
            return Err(QuayError::Io {
                path: src.display().to_string(),
                source,
            })
        }
    };
    if meta.is_dir() {
        // No safe fallback exists for a directory link. `metadata` follows the
        // link, so copying "its contents" here would splice the target's tree
        // into the skill as though it belonged to it — precisely the behaviour
        // this degrade replaces — and would re-enter `copy_missing_rec` ->
        // `copy_symlink` -> here on a link cycle with no depth limit.
        eprintln!(
            "warning: could not recreate symlink {rel} -> {} (Developer Mode or elevation \
             required); the directory link could not be preserved and was skipped",
            link_target.display()
        );
        return Ok(());
    }
    eprintln!(
        "warning: could not recreate symlink {rel} -> {} (Developer Mode or elevation required); \
         copying its contents instead",
        link_target.display()
    );
    std::fs::copy(src, dst)
        .map(|_| ())
        .map_err(|source| QuayError::Io {
            path: dst.display().to_string(),
            source,
        })
}

#[cfg(not(windows))]
fn degrade_symlink_failure(
    _src: &Path,
    _dst: &Path,
    _link_target: &Path,
    _rel: &str,
    e: QuayError,
) -> Result<()> {
    Err(e)
}

#[cfg(unix)]
fn create_symlink_at(_src: &Path, link_target: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(link_target, dst).map_err(|source| QuayError::Io {
        path: dst.display().to_string(),
        source,
    })
}

#[cfg(windows)]
fn create_symlink_at(src: &Path, link_target: &Path, dst: &Path) -> Result<()> {
    // Windows distinguishes a file link from a directory link. `src`'s own
    // metadata (which follows the link) tells us which; a dangling link falls
    // back to a file link.
    let target_is_dir = std::fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false);
    if target_is_dir {
        std::os::windows::fs::symlink_dir(link_target, dst)
    } else {
        std::os::windows::fs::symlink_file(link_target, dst)
    }
    .map_err(|source| QuayError::Io {
        path: dst.display().to_string(),
        source,
    })
}

#[cfg(not(any(unix, windows)))]
fn create_symlink_at(_src: &Path, _link_target: &Path, dst: &Path) -> Result<()> {
    Err(QuayError::Io {
        path: dst.display().to_string(),
        source: std::io::Error::other("symlinks are not supported on this platform"),
    })
}

/// Files in the existing install that the freshly fetched copy does not have.
///
/// Both sides go through `collect_skill_files`, which is the same set `push`,
/// `diff` and mirror-adopt agree on: dotfiles, dot-dirs and symlinks are outside
/// it, so they are never offered for deletion and are always carried forward.
fn compute_extras(dest_dir: &Path, staging: &Path) -> Result<Vec<String>> {
    let fetched: BTreeSet<String> = crate::skill_files::collect_skill_files(staging)?
        .into_iter()
        .collect();
    let mut extras: Vec<String> = crate::skill_files::collect_skill_files(dest_dir)?
        .into_iter()
        .filter(|rel| !fetched.contains(rel))
        .collect();
    // `collect_skill_files` hoists SKILL.md to the front; the prompt wants a
    // plain sorted list.
    extras.sort();
    Ok(extras)
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

    /// Installs `csv-parse` with a single SKILL.md, then returns
    /// (tempdir, files, registry) ready for a second force-add.
    fn installed_fixture() -> (assert_fs::TempDir, FakeFiles, FakeRegistry) {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = make_cfg();
        let body = b"---\nname: csv-parse\ndescription: x.\nversion: 1.0.0\n---\nbody\n".to_vec();
        let files = FakeFiles {
            files: RefCell::new(HashMap::from([("skills/csv-parse/SKILL.md".into(), body)])),
        };
        let regf = FakeRegistry(make_registry("csv-parse", "1.0.0"));
        {
            let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
            mgr.add("csv-parse", None).unwrap();
        }
        (dir, files, regf)
    }

    #[test]
    fn delete_removes_extras() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join("notes.md"), b"my notes").unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_extras("csv-parse", None, true, &|_, _| Ok(ExtraFiles::Delete))
            .unwrap();

        assert!(
            skill_dir.join("SKILL.md").exists(),
            "manifest file must land"
        );
        assert!(
            !skill_dir.join("notes.md").exists(),
            "Delete must drop the extra file"
        );
    }

    #[test]
    fn delete_only_deletes_just_the_named() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join("notes.md"), b"mine").unwrap();
        std::fs::write(skill_dir.join("legacy.md"), b"dropped upstream").unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_extras("csv-parse", None, true, &|_, _| {
            Ok(ExtraFiles::DeleteOnly(vec!["legacy.md".to_string()]))
        })
        .unwrap();

        assert!(!skill_dir.join("legacy.md").exists());
        assert_eq!(std::fs::read(skill_dir.join("notes.md")).unwrap(), b"mine");
    }

    /// The callback crosses a crate boundary. A path it names that was never
    /// offered must not be deleted, however it got there.
    ///
    /// The load-bearing case is a dotfile: `copy_missing_rec` walks it, so a
    /// `skip` entry naming it *does* delete it, but `compute_extras` never
    /// offers it — only the intersection stands between the callback and the
    /// `.quay-mirror` marker that tells `linker.rs` a copy mirror is
    /// quay-managed. (`SKILL.md` and `../sibling.txt` below are covered by
    /// other mechanisms — the `!dst.exists()` check and the fact that `rel` is
    /// built from `file_name()` — so they hold with or without the guard.)
    #[test]
    fn delete_only_ignores_paths_not_offered() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join(".quay-mirror"), b"marker").unwrap();
        std::fs::write(skill_dir.join("notes.md"), b"mine").unwrap();
        let sibling = dir.path().join(".agents/skills/sibling.txt");
        std::fs::write(&sibling, b"not mine to touch").unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_extras("csv-parse", None, true, &|_, extras| {
            assert_eq!(
                extras,
                ["notes.md"],
                "only the non-dotfile extra is ever offered"
            );
            Ok(ExtraFiles::DeleteOnly(vec![
                ".quay-mirror".to_string(),   // walked but never offered
                "notes.md".to_string(),       // genuinely offered
                "SKILL.md".to_string(),       // in the new manifest, never offered
                "../sibling.txt".to_string(), // outside the skill entirely
            ]))
        })
        .unwrap();

        assert!(
            skill_dir.join(".quay-mirror").exists(),
            "a dotfile the user was never asked about must survive being named"
        );
        assert!(
            !skill_dir.join("notes.md").exists(),
            "the one offered path named must still be deleted"
        );
        assert!(skill_dir.join("SKILL.md").exists());
        assert!(sibling.exists(), "a path never offered must survive");
    }

    #[test]
    fn dotfiles_are_never_offered_and_always_survive() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join(".quay-mirror"), b"marker").unwrap();
        std::fs::write(skill_dir.join(".notes.md"), b"private").unwrap();

        let called = RefCell::new(false);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_extras("csv-parse", None, true, &|_, _| {
            *called.borrow_mut() = true;
            Ok(ExtraFiles::Delete)
        })
        .unwrap();

        assert!(
            !*called.borrow(),
            "with only dotfiles present the extra set is empty and the callback must not run"
        );
        assert!(skill_dir.join(".quay-mirror").exists());
        assert!(skill_dir.join(".notes.md").exists());
    }

    /// `notes.md` is load-bearing: with only the symlink present the extra set
    /// is empty, `decide` is never called, and the `Delete` verdict below would
    /// never run — leaving the symlink assertion to pass for the wrong reason.
    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_offered_and_always_survive() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        let target = dir.path().join("outside.txt");
        std::fs::write(&target, b"target").unwrap();
        std::os::unix::fs::symlink(&target, skill_dir.join("link.txt")).unwrap();
        std::fs::write(skill_dir.join("notes.md"), b"mine").unwrap();

        let called = RefCell::new(false);
        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_extras("csv-parse", None, true, &|_, extras| {
            *called.borrow_mut() = true;
            assert!(
                !extras.iter().any(|e| e == "link.txt"),
                "symlinks are outside the managed set: {extras:?}"
            );
            Ok(ExtraFiles::Delete)
        })
        .unwrap();

        assert!(
            *called.borrow(),
            "the decision callback must actually have run"
        );
        assert!(
            !skill_dir.join("notes.md").exists(),
            "the Delete verdict must really have been applied"
        );
        assert!(
            std::fs::symlink_metadata(skill_dir.join("link.txt"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink must survive as a symlink, not be replaced by a regular file"
        );
    }

    /// `is_dir()` follows symlinks, so a directory link used to be recursed
    /// into and the target's contents copied in as though they belonged to the
    /// skill — foreign files imported into the install, and from there into
    /// `quay push`.
    #[cfg(unix)]
    #[test]
    fn a_directory_symlink_is_preserved_and_its_target_never_imported() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        let outside = dir.path().join("outside-docs");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), b"not the skill's").unwrap();
        std::os::unix::fs::symlink(&outside, skill_dir.join("docs")).unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_force("csv-parse", None, true).unwrap();

        let link = skill_dir.join("docs");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the directory link must survive as a link, not become a real directory"
        );
        // Reading through the link still finds the target's file; what must not
        // exist is a real copy of it inside the skill. Drop the link and the
        // path has to go with it.
        std::fs::remove_file(&link).unwrap();
        assert!(
            !skill_dir.join("docs/secret.md").exists(),
            "the target's contents must not have been copied into the skill"
        );
        assert!(
            outside.join("secret.md").exists(),
            "the target is untouched"
        );
    }

    /// Runs on every platform: the classification the Windows degrade path
    /// gates on is plain `io::ErrorKind` matching, so it needs no Windows box
    /// to verify. What actually happens with the classification
    /// (`degrade_symlink_failure`, `#[cfg(windows)]`) cannot be exercised
    /// here.
    #[test]
    fn only_permission_class_symlink_failures_degrade() {
        let permission = QuayError::Io {
            path: "link.txt".into(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert!(is_permission_class_failure(&permission));

        // ERROR_PRIVILEGE_NOT_HELD, whether or not std maps it to a kind.
        let raw_privilege = QuayError::Io {
            path: "link.txt".into(),
            source: std::io::Error::from_raw_os_error(1314),
        };
        assert!(
            is_permission_class_failure(&raw_privilege),
            "the degrade must fire on the raw Windows code even if std leaves it uncategorized"
        );

        let other_raw = QuayError::Io {
            path: "link.txt".into(),
            source: std::io::Error::from_raw_os_error(1315),
        };
        assert!(!is_permission_class_failure(&other_raw));

        let disk_full = QuayError::Io {
            path: "link.txt".into(),
            source: std::io::Error::new(std::io::ErrorKind::StorageFull, "no space"),
        };
        assert!(!is_permission_class_failure(&disk_full));

        let other = QuayError::Io {
            path: "link.txt".into(),
            source: std::io::Error::other("broken object store"),
        };
        assert!(!is_permission_class_failure(&other));
    }

    #[test]
    fn nested_extra_is_deleted_and_leaves_no_empty_dir() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(skill_dir.join("refs/deep")).unwrap();
        std::fs::write(skill_dir.join("refs/deep/x.md"), b"x").unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_extras("csv-parse", None, true, &|_, extras| {
            assert_eq!(extras, ["refs/deep/x.md"]);
            Ok(ExtraFiles::Delete)
        })
        .unwrap();

        assert!(!skill_dir.join("refs/deep/x.md").exists());
        assert!(
            !skill_dir.join("refs").exists(),
            "no husk left where every child was deleted"
        );
    }

    #[test]
    fn callback_error_aborts_before_the_swap() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join("notes.md"), b"mine").unwrap();
        let before = std::fs::read(skill_dir.join("SKILL.md")).unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        let err = mgr
            .add_with_extras("csv-parse", None, true, &|_, _| {
                Err(QuayError::Io {
                    path: "prompt".into(),
                    source: std::io::Error::other("interrupted"),
                })
            })
            .unwrap_err();
        assert!(matches!(err, QuayError::Io { .. }), "got {err:?}");

        assert_eq!(
            std::fs::read(skill_dir.join("SKILL.md")).unwrap(),
            before,
            "an aborted decision must leave the install byte-identical"
        );
        assert_eq!(std::fs::read(skill_dir.join("notes.md")).unwrap(), b"mine");
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

    #[test]
    fn update_one_with_extras_forwards_the_decision() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::write(skill_dir.join("notes.md"), b"mine").unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.update_one_with_extras("csv-parse", &|_, _| Ok(ExtraFiles::Delete))
            .unwrap();

        assert!(!skill_dir.join("notes.md").exists());
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

    #[test]
    fn compute_extras_lists_only_files_missing_from_staging() {
        let dest = assert_fs::TempDir::new().unwrap();
        let staging = assert_fs::TempDir::new().unwrap();

        std::fs::write(dest.path().join("SKILL.md"), b"a").unwrap();
        std::fs::create_dir_all(dest.path().join("refs")).unwrap();
        std::fs::write(dest.path().join("refs/legacy.md"), b"b").unwrap();
        std::fs::write(dest.path().join("notes.md"), b"c").unwrap();

        std::fs::write(staging.path().join("SKILL.md"), b"a-new").unwrap();

        let extras = compute_extras(dest.path(), staging.path()).unwrap();
        assert_eq!(extras, vec!["notes.md", "refs/legacy.md"]);
    }

    #[test]
    fn compute_extras_never_lists_dotfiles() {
        let dest = assert_fs::TempDir::new().unwrap();
        let staging = assert_fs::TempDir::new().unwrap();

        std::fs::write(dest.path().join("SKILL.md"), b"a").unwrap();
        std::fs::write(dest.path().join(".quay-mirror"), b"marker").unwrap();
        std::fs::write(dest.path().join(".notes.md"), b"private").unwrap();
        std::fs::create_dir_all(dest.path().join(".hidden")).unwrap();
        std::fs::write(dest.path().join(".hidden/x.md"), b"x").unwrap();

        std::fs::write(staging.path().join("SKILL.md"), b"a-new").unwrap();

        let extras = compute_extras(dest.path(), staging.path()).unwrap();
        assert!(
            extras.is_empty(),
            "dotfiles and dot-dirs are outside the managed set: {extras:?}"
        );
    }

    #[test]
    fn copy_missing_into_skips_listed_paths() {
        let from = assert_fs::TempDir::new().unwrap();
        let into = assert_fs::TempDir::new().unwrap();

        std::fs::write(from.path().join("keep.md"), b"keep").unwrap();
        std::fs::write(from.path().join("drop.md"), b"drop").unwrap();

        let skip = std::collections::BTreeSet::from(["drop.md".to_string()]);
        copy_missing_into(from.path(), into.path(), &skip).unwrap();

        assert!(into.path().join("keep.md").exists());
        assert!(
            !into.path().join("drop.md").exists(),
            "a skipped file must not be copied forward"
        );
    }

    #[test]
    fn copy_missing_into_leaves_no_empty_dir_when_every_child_is_skipped() {
        let from = assert_fs::TempDir::new().unwrap();
        let into = assert_fs::TempDir::new().unwrap();

        std::fs::create_dir_all(from.path().join("refs")).unwrap();
        std::fs::write(from.path().join("refs/a.md"), b"a").unwrap();
        std::fs::write(from.path().join("refs/b.md"), b"b").unwrap();

        let skip =
            std::collections::BTreeSet::from(["refs/a.md".to_string(), "refs/b.md".to_string()]);
        copy_missing_into(from.path(), into.path(), &skip).unwrap();

        assert!(
            !into.path().join("refs").exists(),
            "a directory whose every file was skipped must not be created"
        );
    }

    /// The counterpart to the husk test above. A directory with no entries at
    /// all cannot be materialized by a child landing in it, so it has to be
    /// recreated explicitly or it is deleted by omission on the path that
    /// promises to keep everything.
    #[test]
    fn copy_missing_into_recreates_a_directory_that_was_already_empty() {
        let from = assert_fs::TempDir::new().unwrap();
        let into = assert_fs::TempDir::new().unwrap();

        std::fs::create_dir_all(from.path().join("scratch")).unwrap();
        std::fs::create_dir_all(from.path().join("refs/deep/empty")).unwrap();
        std::fs::write(from.path().join("refs/deep/x.md"), b"x").unwrap();

        copy_missing_into(from.path(), into.path(), &std::collections::BTreeSet::new()).unwrap();

        assert!(
            into.path().join("scratch").is_dir(),
            "an already-empty directory must survive"
        );
        assert!(
            into.path().join("refs/deep/empty").is_dir(),
            "a nested already-empty directory must survive too"
        );
    }

    /// The user-facing promise: `add --force` with no decision at all — the
    /// `quay-mcp` and non-TTY path — deletes nothing, empty directories
    /// included.
    #[test]
    fn force_add_keeps_an_already_empty_directory() {
        let (dir, files, regf) = installed_fixture();
        let cfg = make_cfg();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(skill_dir.join("scratch")).unwrap();

        let mgr = SkillManager::new(&cfg, &regf, &files, dir.path().to_path_buf());
        mgr.add_with_force("csv-parse", None, true).unwrap();

        assert!(
            skill_dir.join("scratch").is_dir(),
            "an empty local directory must not be dropped by an update"
        );
    }

    #[test]
    fn copy_missing_into_skips_by_nested_posix_key() {
        let from = assert_fs::TempDir::new().unwrap();
        let into = assert_fs::TempDir::new().unwrap();

        std::fs::create_dir_all(from.path().join("refs/deep")).unwrap();
        std::fs::write(from.path().join("refs/deep/x.md"), b"x").unwrap();
        std::fs::write(from.path().join("refs/keep.md"), b"k").unwrap();

        let skip = std::collections::BTreeSet::from(["refs/deep/x.md".to_string()]);
        copy_missing_into(from.path(), into.path(), &skip).unwrap();

        assert!(
            !into.path().join("refs/deep/x.md").exists(),
            "nested skip key must match the POSIX-joined relative path"
        );
        assert!(into.path().join("refs/keep.md").exists());
    }

    #[test]
    fn copy_missing_into_does_not_overwrite_an_existing_file() {
        let from = assert_fs::TempDir::new().unwrap();
        let into = assert_fs::TempDir::new().unwrap();

        std::fs::write(from.path().join("SKILL.md"), b"old").unwrap();
        std::fs::write(into.path().join("SKILL.md"), b"new").unwrap();

        copy_missing_into(from.path(), into.path(), &std::collections::BTreeSet::new()).unwrap();

        assert_eq!(
            std::fs::read(into.path().join("SKILL.md")).unwrap(),
            b"new",
            "the freshly fetched copy always wins"
        );
    }
}
