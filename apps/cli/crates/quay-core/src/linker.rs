//! Project canonical skill directories into mirror locations.
//!
//! `apply_one` is the lowest-level operation: given a canonical skill
//! directory and a mirror config, ensure the mirror exists at the requested
//! path with the requested strategy. Idempotent: re-running on a correctly
//! mirrored skill is a no-op.

use crate::config::{InstallConfig, MirrorConfig, MirrorRoot, MirrorStrategy};
use crate::error::{QuayError, Result};
use std::path::{Path, PathBuf};

/// Outcome of a single `apply_one` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorAction {
    NoOp,
    Created {
        path: PathBuf,
        strategy: MirrorStrategy,
    },
    Replaced {
        path: PathBuf,
        strategy: MirrorStrategy,
    },
    Adopted {
        path: PathBuf,
        strategy: MirrorStrategy,
    },
}

/// One mirror that needs attention, surfaced by [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorDrift {
    pub skill: String,
    pub mirror_path: PathBuf,
    pub reason: String,
}

/// Result of comparing an on-disk mirror path against its canonical skill dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorState {
    /// Nothing exists at the target.
    Missing,
    /// A symlink, or a managed copy whose content matches canonical.
    Correct,
    /// An unmanaged real directory whose content is byte-identical to
    /// canonical — safe to convert to a symlink (no data at risk).
    Adoptable,
    /// A directory whose content differs from canonical (managed copy or
    /// unmanaged real dir). Overwriting it would lose the user's edits.
    Diverged { reason: String },
    /// A path that is neither a directory nor a symlink.
    Conflict { reason: String },
}

/// Compare `target` against `canonical`, reusing the pushed-file content hash.
/// Symlinks are `Correct` here (their target is verified separately by
/// `check()` for configured mirrors).
pub fn classify(target: &Path, canonical: &Path) -> Result<MirrorState> {
    let metadata = match std::fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(MirrorState::Missing),
        Err(e) => {
            return Ok(MirrorState::Conflict {
                reason: format!("cannot stat: {}", e),
            })
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(MirrorState::Correct);
    }
    if !metadata.is_dir() {
        return Ok(MirrorState::Conflict {
            reason: "path exists and is not a directory or symlink".into(),
        });
    }
    let managed = target.join(".quay-mirror").exists();
    let identical = crate::skill_files::pushable_content_hash(target)?
        == crate::skill_files::pushable_content_hash(canonical)?;
    Ok(if !identical {
        MirrorState::Diverged {
            reason: "mirror content differs from canonical".into(),
        }
    } else if managed {
        MirrorState::Correct
    } else if dirs_fully_identical(target, canonical)? {
        MirrorState::Adoptable
    } else {
        // pushable content matches but the tree isn't byte-for-byte equal
        // (extra dotfile, dotdir, or symlink) — adoption would delete it.
        MirrorState::Diverged {
            reason: "mirror content differs from canonical".into(),
        }
    })
}

/// True iff `a` and `b` are the same tree byte-for-byte: identical set of
/// entries at every level, regular files equal by content, symlinks equal by
/// link target, subdirectories recursively equal. Nothing is skipped
/// (dotfiles, `.git`, symlinks all count). Used to gate adoption, which
/// DELETES the target — anything present in the target but not in canonical
/// (a hand-added dotfile, a symlink, an extra file) must block adoption.
fn dirs_fully_identical(a: &Path, b: &Path) -> Result<bool> {
    use std::collections::BTreeMap;
    fn entries(dir: &Path) -> Result<BTreeMap<std::ffi::OsString, std::fs::FileType>> {
        let mut map = BTreeMap::new();
        for entry in std::fs::read_dir(dir).map_err(|source| QuayError::Io {
            path: dir.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| QuayError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let ft = entry.file_type().map_err(|source| QuayError::Io {
                path: entry.path().display().to_string(),
                source,
            })?;
            map.insert(entry.file_name(), ft);
        }
        Ok(map)
    }
    let (ea, eb) = (entries(a)?, entries(b)?);
    if ea.len() != eb.len() {
        return Ok(false);
    }
    for (name, ft_a) in &ea {
        let ft_b = match eb.get(name) {
            Some(ft) => ft,
            None => return Ok(false),
        };
        let (pa, pb) = (a.join(name), b.join(name));
        if ft_a.is_symlink() || ft_b.is_symlink() {
            if !ft_a.is_symlink() || !ft_b.is_symlink() {
                return Ok(false);
            }
            let (la, lb) = (
                std::fs::read_link(&pa).map_err(|source| QuayError::Io {
                    path: pa.display().to_string(),
                    source,
                })?,
                std::fs::read_link(&pb).map_err(|source| QuayError::Io {
                    path: pb.display().to_string(),
                    source,
                })?,
            );
            if la != lb {
                return Ok(false);
            }
        } else if ft_a.is_dir() || ft_b.is_dir() {
            if !ft_a.is_dir() || !ft_b.is_dir() {
                return Ok(false);
            }
            if !dirs_fully_identical(&pa, &pb)? {
                return Ok(false);
            }
        } else {
            let (ca, cb) = (
                std::fs::read(&pa).map_err(|source| QuayError::Io {
                    path: pa.display().to_string(),
                    source,
                })?,
                std::fs::read(&pb).map_err(|source| QuayError::Io {
                    path: pb.display().to_string(),
                    source,
                })?,
            );
            if ca != cb {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Apply a single mirror entry for one skill.
///
/// When `adopt` is set, an `Adoptable` mirror — an unmanaged dir byte-identical
/// to canonical — is converted to a managed mirror (a symlink on most
/// platforms; the mirror's configured strategy); otherwise it is a no-op.
pub fn apply_one(
    canonical_skill_dir: &Path,
    mirror_root: &Path,
    skill_name: &str,
    mirror: &MirrorConfig,
    force: bool,
    adopt: bool,
) -> Result<MirrorAction> {
    let target = mirror_root.join(skill_name);
    let strategy = resolve_strategy(mirror.strategy);

    std::fs::create_dir_all(mirror_root).map_err(|source| QuayError::Io {
        path: mirror_root.display().to_string(),
        source,
    })?;

    match classify(&target, canonical_skill_dir)? {
        MirrorState::Missing => {
            create_mirror(canonical_skill_dir, &target, strategy)?;
            Ok(MirrorAction::Created {
                path: target,
                strategy,
            })
        }
        MirrorState::Correct => Ok(MirrorAction::NoOp),
        MirrorState::Adoptable => {
            if adopt {
                // content is byte-identical, so replacing with a managed mirror loses nothing
                replace_mirror(canonical_skill_dir, &target, strategy)?;
                Ok(MirrorAction::Adopted {
                    path: target,
                    strategy,
                })
            } else {
                Ok(MirrorAction::NoOp)
            }
        }
        MirrorState::Diverged { reason } => {
            if !force {
                return Err(QuayError::MirrorConflict {
                    path: target.display().to_string(),
                    reason: format!(
                        "{reason}\n  keep it: copy your edit to {} then re-run\n  discard: re-run with --force",
                        canonical_skill_dir.display()
                    ),
                });
            }
            replace_mirror(canonical_skill_dir, &target, strategy)?;
            Ok(MirrorAction::Replaced {
                path: target,
                strategy,
            })
        }
        MirrorState::Conflict { reason } => {
            if !force {
                return Err(QuayError::MirrorConflict {
                    path: target.display().to_string(),
                    reason,
                });
            }
            replace_mirror(canonical_skill_dir, &target, strategy)?;
            Ok(MirrorAction::Replaced {
                path: target,
                strategy,
            })
        }
    }
}

/// Apply every mirror in `install.mirrors` for a single skill.
pub fn apply_all(
    install: &InstallConfig,
    project_root: &Path,
    skill_name: &str,
    force: bool,
) -> Result<Vec<MirrorAction>> {
    let canonical_skill_dir = project_root.join(&install.canonical).join(skill_name);
    if !canonical_skill_dir.exists() {
        return Err(QuayError::MirrorCheckFailed(format!(
            "canonical skill not found at {}",
            canonical_skill_dir.display()
        )));
    }
    let mut actions = Vec::with_capacity(install.mirrors.len());
    for mirror in &install.mirrors {
        let mirror_root = project_root.join(&mirror.path);
        let action = apply_one(
            &canonical_skill_dir,
            &mirror_root,
            skill_name,
            mirror,
            force,
            matches!(install.auto_link, Some(true)),
        )?;
        actions.push(action);
    }
    Ok(actions)
}

/// Verify every configured mirror exists and points at the right canonical.
pub fn check(
    install: &InstallConfig,
    project_root: &Path,
    skill_names: &[String],
) -> Result<Vec<MirrorDrift>> {
    let mut drift = Vec::new();
    for name in skill_names {
        let canonical = project_root.join(&install.canonical).join(name);
        for mirror in &install.mirrors {
            let target = project_root.join(&mirror.path).join(name);
            match classify(&target, &canonical)? {
                MirrorState::Missing => drift.push(MirrorDrift {
                    skill: name.clone(),
                    mirror_path: target,
                    reason: "mirror missing".into(),
                }),
                MirrorState::Diverged { reason } | MirrorState::Conflict { reason } => {
                    drift.push(MirrorDrift {
                        skill: name.clone(),
                        mirror_path: target,
                        reason,
                    })
                }
                MirrorState::Adoptable => drift.push(MirrorDrift {
                    skill: name.clone(),
                    mirror_path: target,
                    reason: "unmanaged directory; run `quay link` to adopt".into(),
                }),
                MirrorState::Correct => {
                    // Verify a symlink still points at canonical (copies already
                    // content-checked by classify).
                    if let Ok(linked) = std::fs::read_link(&target) {
                        let resolved = if linked.is_absolute() {
                            linked
                        } else {
                            target
                                .parent()
                                .map(|p| p.join(&linked))
                                .unwrap_or(linked.clone())
                        };
                        if !same_path(&resolved, &canonical) {
                            drift.push(MirrorDrift {
                                skill: name.clone(),
                                mirror_path: target,
                                reason: format!(
                                    "symlink target {} != canonical {}",
                                    resolved.display(),
                                    canonical.display()
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(drift)
}

/// Outcome of a discovery-driven reconcile across all known mirror roots.
#[derive(Debug, Default)]
pub struct ReconcileReport {
    /// (skill, mirror_path, action) for every mirror that was created/adopted/etc.
    pub actions: Vec<(String, PathBuf, MirrorAction)>,
    /// Diverged mirrors that were not overwritten (no `force`).
    pub diverged: Vec<MirrorDrift>,
    /// (skill, target) unmanaged-but-identical dirs awaiting the adopt opt-in.
    pub needs_optin: Vec<(String, PathBuf)>,
}

/// The mirror roots to reconcile: configured mirrors ∪ known roots present on
/// disk, deduped, canonical excluded. Empty when the canonical dir is absent
/// (a lone tool dir is the source of truth — nothing to reconcile). Pure
/// read-only discovery: performs no filesystem writes.
pub fn discover_roots(
    install: &InstallConfig,
    project_root: &Path,
) -> Vec<(PathBuf, MirrorStrategy)> {
    let mut roots = Vec::new();
    if !project_root.join(&install.canonical).exists() {
        return roots;
    }
    let canonical_str = install.canonical.to_string_lossy();
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for m in &install.mirrors {
        if seen.insert(m.path.clone()) {
            roots.push((m.path.clone(), m.strategy));
        }
    }
    for root in MirrorRoot::all() {
        let rel = PathBuf::from(root.dir());
        if rel.to_string_lossy() == canonical_str {
            continue;
        }
        if project_root.join(&rel).exists() && seen.insert(rel.clone()) {
            roots.push((rel, MirrorStrategy::Auto));
        }
    }
    roots
}

/// Scan every known mirror root on disk (not just `install.mirrors`) and
/// reconcile each skill against canonical. No-op when the canonical dir is
/// absent (a lone tool dir is the source of truth — do not invent a canonical).
///
/// Mutating: creates/adopts/replaces mirrors on disk. For a read-only check,
/// use [`discover_roots`] + [`classify`] instead.
pub fn reconcile(
    install: &InstallConfig,
    project_root: &Path,
    skill_names: &[String],
    force: bool,
) -> Result<ReconcileReport> {
    let mut report = ReconcileReport::default();
    let canonical_root = project_root.join(&install.canonical);
    if !canonical_root.exists() {
        return Ok(report);
    }
    let adopt = matches!(install.auto_link, Some(true));
    let roots = discover_roots(install, project_root);

    for name in skill_names {
        let canonical_skill = canonical_root.join(name);
        if !canonical_skill.exists() {
            continue;
        }
        for (rel, strategy) in &roots {
            let mirror_root = project_root.join(rel);
            let target = mirror_root.join(name);
            match classify(&target, &canonical_skill)? {
                MirrorState::Adoptable if !adopt => {
                    report.needs_optin.push((name.clone(), target));
                }
                MirrorState::Diverged { reason } | MirrorState::Conflict { reason } if !force => {
                    report.diverged.push(MirrorDrift {
                        skill: name.clone(),
                        mirror_path: target,
                        reason,
                    });
                }
                // Missing / Correct / Adoptable(adopt) / Diverged(force) / Conflict(force):
                // let apply_one perform the create / adopt / replace / refusal.
                _ => {
                    let mirror = MirrorConfig {
                        path: rel.clone(),
                        strategy: *strategy,
                    };
                    let action =
                        apply_one(&canonical_skill, &mirror_root, name, &mirror, force, adopt)?;
                    if !matches!(action, MirrorAction::NoOp) {
                        report.actions.push((name.clone(), target, action));
                    }
                }
            }
        }
    }
    Ok(report)
}

/// Resolve `Auto` to the platform-best concrete strategy.
pub fn resolve_strategy(strategy: MirrorStrategy) -> MirrorStrategy {
    match strategy {
        MirrorStrategy::Auto => {
            if cfg!(unix) {
                MirrorStrategy::Symlink
            } else if cfg!(windows) {
                MirrorStrategy::Junction
            } else {
                MirrorStrategy::Copy
            }
        }
        other => other,
    }
}

fn create_mirror(canonical: &Path, target: &Path, strategy: MirrorStrategy) -> Result<()> {
    match strategy {
        MirrorStrategy::Symlink => create_symlink(canonical, target),
        MirrorStrategy::Junction => create_junction(canonical, target),
        MirrorStrategy::Copy => create_copy(canonical, target),
        MirrorStrategy::Auto => unreachable!("resolve_strategy must be called first"),
    }
}

fn replace_mirror(canonical: &Path, target: &Path, strategy: MirrorStrategy) -> Result<()> {
    let metadata = std::fs::symlink_metadata(target).map_err(|source| QuayError::Io {
        path: target.display().to_string(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        // Windows distinguishes the two: a directory symlink or junction — which
        // is what every mirror is — unlinks with remove_dir, and remove_file
        // fails on it. On unix remove_file handles both, so try it first and
        // fall back rather than cfg-splitting.
        std::fs::remove_file(target)
            .or_else(|_| std::fs::remove_dir(target))
            .map_err(|source| QuayError::Io {
                path: target.display().to_string(),
                source,
            })?;
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(target).map_err(|source| QuayError::Io {
            path: target.display().to_string(),
            source,
        })?;
    } else {
        std::fs::remove_file(target).map_err(|source| QuayError::Io {
            path: target.display().to_string(),
            source,
        })?;
    }
    create_mirror(canonical, target, strategy)
}

#[cfg(unix)]
fn create_symlink(canonical: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(canonical, target).map_err(|source| QuayError::Io {
        path: target.display().to_string(),
        source,
    })
}

#[cfg(windows)]
fn create_symlink(canonical: &Path, target: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(canonical, target).map_err(|source| QuayError::Io {
        path: target.display().to_string(),
        source,
    })
}

#[cfg(not(any(unix, windows)))]
fn create_symlink(_canonical: &Path, _target: &Path) -> Result<()> {
    Err(QuayError::UnsupportedStrategy {
        strategy: "symlink".into(),
    })
}

fn create_junction(canonical: &Path, target: &Path) -> Result<()> {
    create_symlink(canonical, target)
}

fn create_copy(canonical: &Path, target: &Path) -> Result<()> {
    copy_dir_recursive(canonical, target)?;
    std::fs::write(target.join(".quay-mirror"), b"").map_err(|source| QuayError::Io {
        path: target.display().to_string(),
        source,
    })?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|source| QuayError::Io {
        path: dst.display().to_string(),
        source,
    })?;
    for entry in std::fs::read_dir(src).map_err(|source| QuayError::Io {
        path: src.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| QuayError::Io {
            path: src.display().to_string(),
            source,
        })?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|source| QuayError::Io {
                path: to.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canon_a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let canon_b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    canon_a == canon_b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{InstallConfig, MirrorConfig, MirrorStrategy};
    use assert_fs::prelude::*;

    fn project_with_skill(skill: &str) -> assert_fs::TempDir {
        let dir = assert_fs::TempDir::new().unwrap();
        let canonical = dir.child(format!(".agents/skills/{}", skill));
        std::fs::create_dir_all(canonical.path()).unwrap();
        std::fs::write(canonical.path().join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        dir
    }

    #[test]
    fn apply_one_creates_symlink_when_target_missing() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        let actions = apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], MirrorAction::Created { .. }));
        let mirror = dir.path().join(".claude/skills/csv-parse");
        assert!(mirror.exists());
        assert!(std::fs::symlink_metadata(&mirror)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn apply_one_is_idempotent_for_existing_correct_mirror() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        let actions = apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        assert_eq!(actions, vec![MirrorAction::NoOp]);
    }

    #[test]
    fn apply_one_errors_on_conflict_without_force() {
        let dir = project_with_skill("csv-parse");
        let conflict = dir.path().join(".claude/skills/csv-parse");
        std::fs::create_dir_all(&conflict).unwrap();
        std::fs::write(conflict.join("user-file.md"), b"theirs").unwrap();

        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        let err = apply_all(&install, dir.path(), "csv-parse", false).unwrap_err();
        assert!(matches!(err, QuayError::MirrorConflict { .. }));
        assert!(conflict.join("user-file.md").exists());
    }

    #[test]
    fn apply_one_replaces_when_forced() {
        let dir = project_with_skill("csv-parse");
        let conflict = dir.path().join(".claude/skills/csv-parse");
        std::fs::create_dir_all(&conflict).unwrap();
        std::fs::write(conflict.join("user-file.md"), b"theirs").unwrap();

        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        let actions = apply_all(&install, dir.path(), "csv-parse", true).unwrap();
        assert!(matches!(actions[0], MirrorAction::Replaced { .. }));
        let mirror = dir.path().join(".claude/skills/csv-parse");
        assert!(std::fs::symlink_metadata(&mirror)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn apply_one_with_copy_strategy_creates_directory_copy() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".cursor/rules".into(),
                strategy: MirrorStrategy::Copy,
            }],
            auto_link: None,
        };
        let actions = apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        assert!(matches!(actions[0], MirrorAction::Created { .. }));
        let mirror = dir.path().join(".cursor/rules/csv-parse");
        assert!(mirror.is_dir());
        assert!(mirror.join("SKILL.md").exists());
        assert!(mirror.join(".quay-mirror").exists());
    }

    #[test]
    fn check_reports_drift_when_mirror_missing() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        let drift = check(&install, dir.path(), &["csv-parse".to_string()]).unwrap();
        assert_eq!(drift.len(), 1);
        assert!(drift[0].reason.contains("missing"));
    }

    #[test]
    fn check_clean_when_mirror_correct() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        let drift = check(&install, dir.path(), &["csv-parse".to_string()]).unwrap();
        assert!(drift.is_empty());
    }

    #[test]
    fn resolve_strategy_auto_picks_platform_default() {
        let resolved = resolve_strategy(MirrorStrategy::Auto);
        if cfg!(unix) {
            assert_eq!(resolved, MirrorStrategy::Symlink);
        } else if cfg!(windows) {
            assert_eq!(resolved, MirrorStrategy::Junction);
        }
    }

    #[test]
    fn resolve_strategy_passes_concrete_through() {
        assert_eq!(resolve_strategy(MirrorStrategy::Copy), MirrorStrategy::Copy);
    }

    fn write_copy_mirror(
        dir: &assert_fs::TempDir,
        root: &str,
        skill: &str,
        body: &[u8],
    ) -> std::path::PathBuf {
        let m = dir.path().join(root).join(skill);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), body).unwrap();
        std::fs::write(m.join(".quay-mirror"), b"").unwrap();
        m
    }

    #[test]
    fn classify_managed_copy_identical_is_correct() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        // identical body to project_with_skill's canonical
        let m = write_copy_mirror(&dir, ".codex/skills", "csv-parse", b"---\nname: x\n---\n");
        assert_eq!(classify(&m, &canonical).unwrap(), MirrorState::Correct);
    }

    #[test]
    fn classify_managed_copy_edited_is_diverged() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        let m = write_copy_mirror(&dir, ".codex/skills", "csv-parse", b"EDITED IN MIRROR");
        assert!(matches!(
            classify(&m, &canonical).unwrap(),
            MirrorState::Diverged { .. }
        ));
    }

    #[test]
    fn classify_unmanaged_dir_identical_is_adoptable() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap(); // no .quay-mirror marker
        assert_eq!(classify(&m, &canonical).unwrap(), MirrorState::Adoptable);
    }

    #[test]
    fn classify_unmanaged_dir_edited_is_diverged() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"DIFFERENT").unwrap();
        assert!(matches!(
            classify(&m, &canonical).unwrap(),
            MirrorState::Diverged { .. }
        ));
    }

    #[test]
    fn check_reports_copy_content_drift() {
        let dir = project_with_skill("csv-parse");
        // configure a copy mirror, materialize it, then edit it
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".codex/skills".into(),
                strategy: MirrorStrategy::Copy,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        let edited = dir.path().join(".codex/skills/csv-parse/SKILL.md");
        std::fs::write(&edited, b"HAND EDITED").unwrap();
        let drift = check(&install, dir.path(), &["csv-parse".to_string()]).unwrap();
        assert_eq!(drift.len(), 1);
        assert!(
            drift[0].reason.contains("content differs"),
            "got: {}",
            drift[0].reason
        );
    }

    #[test]
    fn check_no_false_drift_for_symlink() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        let drift = check(&install, dir.path(), &["csv-parse".to_string()]).unwrap();
        assert!(drift.is_empty(), "symlink must not drift, got: {:?}", drift);
    }

    #[test]
    fn apply_refuses_to_overwrite_diverged_copy_without_force() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".codex/skills".into(),
                strategy: MirrorStrategy::Copy,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        let edited = dir.path().join(".codex/skills/csv-parse/SKILL.md");
        std::fs::write(&edited, b"MY EDIT").unwrap();

        let err = apply_all(&install, dir.path(), "csv-parse", false).unwrap_err();
        assert!(matches!(err, QuayError::MirrorConflict { .. }));
        // edit survives
        assert_eq!(std::fs::read(&edited).unwrap(), b"MY EDIT");
    }

    #[test]
    fn apply_overwrites_diverged_copy_with_force() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".codex/skills".into(),
                strategy: MirrorStrategy::Copy,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        let edited = dir.path().join(".codex/skills/csv-parse/SKILL.md");
        std::fs::write(&edited, b"MY EDIT").unwrap();

        let actions = apply_all(&install, dir.path(), "csv-parse", true).unwrap();
        assert!(matches!(actions[0], MirrorAction::Replaced { .. }));
        assert_eq!(std::fs::read(&edited).unwrap(), b"---\nname: x\n---\n"); // back to canonical
    }

    #[test]
    fn apply_adopts_unmanaged_identical_dir_when_opted_in() {
        let dir = project_with_skill("csv-parse");
        // unmanaged real dir identical to canonical, no marker
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();

        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".codex/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: Some(true),
        };
        let actions = apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        assert!(
            matches!(actions[0], MirrorAction::Adopted { .. }),
            "got: {:?}",
            actions[0]
        );
        assert!(std::fs::symlink_metadata(&m)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn apply_leaves_unmanaged_identical_dir_when_not_opted_in() {
        let dir = project_with_skill("csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();

        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".codex/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        let actions = apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        assert_eq!(actions, vec![MirrorAction::NoOp]);
        assert!(std::fs::symlink_metadata(&m).unwrap().file_type().is_dir()); // still a real dir
    }

    #[test]
    fn apply_refuses_symlink_replaced_by_edited_dir_without_force() {
        let dir = project_with_skill("csv-parse");
        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".claude/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: None,
        };
        apply_all(&install, dir.path(), "csv-parse", false).unwrap();
        // replace the symlink with a real, edited directory
        let m = dir.path().join(".claude/skills/csv-parse");
        // A directory symlink needs remove_dir on Windows; remove_file only
        // unlinks it on unix.
        if std::fs::remove_file(&m).is_err() {
            std::fs::remove_dir(&m).unwrap();
        }
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"EDITED AFTER UNLINK").unwrap();

        let err = apply_all(&install, dir.path(), "csv-parse", false).unwrap_err();
        assert!(matches!(err, QuayError::MirrorConflict { .. }));
        assert_eq!(
            std::fs::read(m.join("SKILL.md")).unwrap(),
            b"EDITED AFTER UNLINK"
        );
    }

    #[test]
    fn reconcile_noop_when_no_canonical() {
        let dir = assert_fs::TempDir::new().unwrap();
        // a lone .claude with no .agents canonical
        let m = dir.path().join(".claude/skills/foo");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"solo").unwrap();
        let install = InstallConfig::default();
        let report = reconcile(&install, dir.path(), &["foo".to_string()], false).unwrap();
        assert!(
            report.actions.is_empty()
                && report.diverged.is_empty()
                && report.needs_optin.is_empty()
        );
        assert!(std::fs::symlink_metadata(&m).unwrap().file_type().is_dir()); // untouched
    }

    #[test]
    fn reconcile_discovers_unconfigured_root_and_flags_optin() {
        let dir = project_with_skill("csv-parse");
        // someone added .codex, not in config, identical content, no marker
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        let install = InstallConfig::default(); // no mirrors, auto_link = None
        let report = reconcile(&install, dir.path(), &["csv-parse".to_string()], false).unwrap();
        assert_eq!(report.needs_optin.len(), 1);
        assert_eq!(report.needs_optin[0].1, m);
        assert!(std::fs::symlink_metadata(&m).unwrap().file_type().is_dir()); // not adopted yet
    }

    #[test]
    fn reconcile_adopts_discovered_root_when_opted_in() {
        let dir = project_with_skill("csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        let install = InstallConfig {
            auto_link: Some(true),
            ..InstallConfig::default()
        };
        let report = reconcile(&install, dir.path(), &["csv-parse".to_string()], false).unwrap();
        assert!(report.needs_optin.is_empty());
        assert!(report
            .actions
            .iter()
            .any(|(_, _, a)| matches!(a, MirrorAction::Adopted { .. })));
        assert!(std::fs::symlink_metadata(&m)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn classify_unmanaged_dir_with_extra_dotfile_is_not_adoptable() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        std::fs::write(m.join(".notes.md"), b"hand written notes").unwrap();
        assert!(
            matches!(
                classify(&m, &canonical).unwrap(),
                MirrorState::Diverged { .. }
            ),
            "unmanaged dir with an extra dotfile must not be classified Adoptable"
        );
    }

    #[test]
    fn apply_does_not_delete_extra_files_when_adopting() {
        let dir = project_with_skill("csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        std::fs::write(m.join(".notes.md"), b"hand written notes").unwrap();

        let install = InstallConfig {
            canonical: ".agents/skills".into(),
            mirrors: vec![MirrorConfig {
                path: ".codex/skills".into(),
                strategy: MirrorStrategy::Symlink,
            }],
            auto_link: Some(true),
        };
        // Diverged without force is a refusal (Err), not a silent no-op — the
        // point under test is that nothing on disk was touched either way.
        assert!(matches!(
            apply_all(&install, dir.path(), "csv-parse", false).unwrap_err(),
            QuayError::MirrorConflict { .. }
        ));

        assert!(
            m.join(".notes.md").exists(),
            "hand-created file must survive"
        );
        assert!(
            std::fs::symlink_metadata(&m).unwrap().file_type().is_dir(),
            "target must remain a real directory, not be replaced with a symlink"
        );
    }

    #[test]
    fn reconcile_flags_discovered_diverged_root() {
        let dir = project_with_skill("csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"HAND WRITTEN, DIFFERENT").unwrap();
        let install = InstallConfig::default();
        let report = reconcile(&install, dir.path(), &["csv-parse".to_string()], false).unwrap();
        assert_eq!(report.diverged.len(), 1);
        assert!(report.diverged[0].reason.contains("content differs"));
        assert_eq!(
            std::fs::read(m.join("SKILL.md")).unwrap(),
            b"HAND WRITTEN, DIFFERENT"
        ); // not clobbered
    }

    #[test]
    fn classify_conflict_when_target_is_plain_file() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        let target = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"not a directory").unwrap();
        assert!(matches!(
            classify(&target, &canonical).unwrap(),
            MirrorState::Conflict { .. }
        ));
    }

    #[test]
    fn classify_nested_subdir_identical_is_adoptable() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(canonical.join("scripts")).unwrap();
        std::fs::write(canonical.join("scripts/run.sh"), b"echo hi\n").unwrap();

        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(m.join("scripts")).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        std::fs::write(m.join("scripts/run.sh"), b"echo hi\n").unwrap();

        assert_eq!(classify(&m, &canonical).unwrap(), MirrorState::Adoptable);
    }

    #[test]
    fn classify_nested_subdir_differs_is_diverged() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(canonical.join("scripts")).unwrap();
        std::fs::write(canonical.join("scripts/run.sh"), b"echo hi\n").unwrap();

        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(m.join("scripts")).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        std::fs::write(m.join("scripts/run.sh"), b"echo bye\n").unwrap();

        assert!(matches!(
            classify(&m, &canonical).unwrap(),
            MirrorState::Diverged { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn classify_unmanaged_dir_with_extra_symlink_is_not_adoptable() {
        let dir = project_with_skill("csv-parse");
        let canonical = dir.path().join(".agents/skills/csv-parse");
        let m = dir.path().join(".codex/skills/csv-parse");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("SKILL.md"), b"---\nname: x\n---\n").unwrap();
        std::os::unix::fs::symlink(canonical.join("SKILL.md"), m.join("extra-link")).unwrap();

        assert!(
            matches!(
                classify(&m, &canonical).unwrap(),
                MirrorState::Diverged { .. }
            ),
            "unmanaged dir with an extra symlink must not be classified Adoptable"
        );
    }
}
