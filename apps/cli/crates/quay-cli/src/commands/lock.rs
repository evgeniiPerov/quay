//! `quay lock` — generate / check / heal / sync `skills-lock.json`.

use quay_core::lock::{self, LockEntry, SkillsLock, SourceType};
use quay_core::lock_hash::folder_hash;
use quay_core::push_log::PushLog;
use quay_core::scanner::{scan_local, LocalSkill};
use std::path::Path;

/// Build a `SkillsLock` reconciling the skills on disk with the existing
/// lockfile.
///
/// Provenance is preserved: for a skill that is already tracked, the prior
/// `source` / `sourceType` / `skillPath` are kept and only `computedHash` is
/// refreshed from disk. This is essential for interop — a `github`/`git` entry
/// written by quay or by the `skills` CLI must survive a plain `quay lock`,
/// `--heal`, or any add/remove/update regenerate rather than being flattened
/// to `local`. A skill on disk with no prior entry can't have its origin known
/// from a scan, so it is recorded as `SourceType::Local`. Entries for skills no
/// longer on disk are dropped (only on-disk skills are iterated).
fn build_lock_from_disk(
    project_root: &Path,
    skills: &[LocalSkill],
) -> Result<SkillsLock, Box<dyn std::error::Error>> {
    let existing = lock::read(project_root)?;
    let mut lock = SkillsLock::empty();
    for s in skills {
        let skill_md = s.canonical_path();
        let folder = skill_md.parent().unwrap_or(skill_md);
        // A failed hash would make `--check` report this skill as perpetually
        // drifted, so surface the cause instead of silently writing an empty hash.
        let hash = folder_hash(folder)?;
        let entry = match existing.as_ref().and_then(|e| e.skills.get(&s.meta.name)) {
            // Already tracked — keep its provenance, refresh only the hash.
            Some(prev) => LockEntry {
                source: prev.source.clone(),
                source_type: prev.source_type,
                skill_path: prev.skill_path.clone(),
                computed_hash: hash,
                // Preserve any vercel-only keys (ref/subagents/...) verbatim.
                extra: prev.extra.clone(),
            },
            // New to the lockfile — origin unknown from a scan, default to local.
            None => {
                let rel = skill_md
                    .strip_prefix(project_root)
                    .unwrap_or(skill_md)
                    .to_string_lossy()
                    .replace('\\', "/");
                LockEntry {
                    source: rel.clone(),
                    source_type: SourceType::Local,
                    skill_path: Some(rel),
                    computed_hash: hash,
                    extra: Default::default(),
                }
            }
        };
        lock.skills.insert(s.meta.name.clone(), entry);
    }
    Ok(lock)
}

/// Best-effort lockfile refresh for mutating commands (add/remove/update):
/// regenerate `skills-lock.json` if the project already uses one, warning on
/// failure rather than failing the command. The primary operation has already
/// succeeded and printed its result by the time this runs, so a lockfile hiccup
/// (e.g. read-only lockfile, transient IO) must not flip the exit code.
pub fn regenerate_if_present(project_root: &Path) {
    if !project_root.join(lock::LOCKFILE_NAME).exists() {
        return;
    }
    if let Err(e) = regenerate(project_root) {
        eprintln!("warning: failed to update {}: {e}", lock::LOCKFILE_NAME);
    }
}

/// Load the push log, warning (not failing) on any load error.
/// `PushLog::load` already returns an empty log when the file is absent, so a
/// real `Err` here means corruption or an IO/migration failure worth surfacing.
fn load_push_log(project_root: &Path) -> PushLog {
    let config_dir = crate::config_io::default_config_dir();
    PushLog::load(
        config_dir.as_deref().unwrap_or(project_root),
        Some(project_root),
    )
    .unwrap_or_else(|e| {
        eprintln!("warning: could not read push log ({e}); treating as empty");
        PushLog::default()
    })
}

/// Regenerate `skills-lock.json` from the current on-disk scan. Returns the
/// number of skills written. Callers that must not fail on a lockfile error
/// should use [`regenerate_if_present`] instead.
pub fn regenerate(project_root: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let push_log = load_push_log(project_root);
    let skills = scan_local(project_root, &push_log);
    let lock = build_lock_from_disk(project_root, &skills)?;
    lock::write_atomic(project_root, &lock)?;
    Ok(lock.skills.len())
}

pub fn run(
    project_root: &Path,
    check: bool,
    heal: bool,
    sync: bool,
    online: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if online {
        eprintln!("note: --online is not yet implemented; performing offline check only");
    }
    let push_log = load_push_log(project_root);
    let skills = scan_local(project_root, &push_log);

    if check {
        return check_impl(project_root, &skills, online);
    }
    if sync {
        return sync_impl(project_root);
    }
    if heal {
        let drift = compute_drift(project_root, &skills)?;
        for d in &drift {
            println!("healing {}", d.line());
        }
        let lock = build_lock_from_disk(project_root, &skills)?;
        lock::write_atomic(project_root, &lock)?;
        println!(
            "healed {} ({} skills)",
            lock::LOCKFILE_NAME,
            lock.skills.len()
        );
    } else {
        let lock = build_lock_from_disk(project_root, &skills)?;
        lock::write_atomic(project_root, &lock)?;
        println!(
            "wrote {} ({} skills)",
            lock::LOCKFILE_NAME,
            lock.skills.len()
        );
    }
    Ok(())
}

/// The lockfile JSON string for a source type, for user-facing messages
/// (matches the on-disk `sourceType` value rather than the Rust enum name).
fn source_type_label(t: SourceType) -> &'static str {
    match t {
        SourceType::Github => "github",
        SourceType::Git => "git",
        SourceType::Local => "local",
        SourceType::WellKnown => "well-known",
        SourceType::NodeModules => "node-modules",
    }
}

/// One drift finding between the lockfile and what's on disk.
enum Drift {
    Missing(String),   // in lock, no file on disk
    Untracked(String), // on disk, not in lock
    Modified(String),  // hash differs
}

impl Drift {
    fn line(&self) -> String {
        match self {
            Drift::Missing(n) => format!("missing: {n} (in lockfile, not on disk)"),
            Drift::Untracked(n) => format!("untracked: {n} (on disk, not in lockfile)"),
            Drift::Modified(n) => format!("modified: {n} (content differs from lockfile)"),
        }
    }
}

/// Classify every skill as in-sync or drifted against the lockfile.
fn compute_drift(
    project_root: &Path,
    skills: &[LocalSkill],
) -> Result<Vec<Drift>, Box<dyn std::error::Error>> {
    let lock = lock::read(project_root)?.unwrap_or_else(SkillsLock::empty);
    let mut drift = Vec::new();
    let mut on_disk = std::collections::BTreeSet::new();
    for s in skills {
        on_disk.insert(s.meta.name.clone());
        match lock.skills.get(&s.meta.name) {
            None => drift.push(Drift::Untracked(s.meta.name.clone())),
            Some(entry) => {
                let folder = s
                    .canonical_path()
                    .parent()
                    .unwrap_or_else(|| s.canonical_path());
                let hash = folder_hash(folder)?;
                if hash != entry.computed_hash {
                    drift.push(Drift::Modified(s.meta.name.clone()));
                }
            }
        }
    }
    for name in lock.skills.keys() {
        if !on_disk.contains(name) {
            drift.push(Drift::Missing(name.clone()));
        }
    }
    Ok(drift)
}

fn check_impl(
    project_root: &Path,
    skills: &[LocalSkill],
    _online: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let drift = compute_drift(project_root, skills)?;
    if drift.is_empty() {
        println!("{} is in sync", lock::LOCKFILE_NAME);
        return Ok(());
    }
    for d in &drift {
        eprintln!("{}", d.line());
    }
    Err(format!("{} drift finding(s); run `quay lock --heal`", drift.len()).into())
}

fn sync_impl(project_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let lock_data = match lock::read(project_root)? {
        Some(l) => l,
        None => {
            println!("no {} to sync", lock::LOCKFILE_NAME);
            return Ok(());
        }
    };

    // Build the set of skill names that are already present on disk.
    let push_log = load_push_log(project_root);
    let present: std::collections::BTreeSet<String> = scan_local(project_root, &push_log)
        .into_iter()
        .map(|s| s.meta.name)
        .collect();

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for (name, entry) in &lock_data.skills {
        if present.contains(name) {
            // Already on disk — nothing to do.
            continue;
        }
        match entry.source_type {
            SourceType::Github | SourceType::Git => {
                // Convert source to a full git clone URL.
                let hub_url = match entry.source_type {
                    SourceType::Github => {
                        format!("https://github.com/{}.git", entry.source)
                    }
                    _ => entry.source.clone(),
                };
                println!("installing {} from {}", name, hub_url);
                match crate::commands::add::install_from_url(name, &hub_url, project_root) {
                    Ok(()) => {
                        println!("  installed {}", name);
                        installed += 1;
                    }
                    Err(e) => {
                        eprintln!("  error installing {}: {}", name, e);
                        // A real failure is distinct from an intentional skip.
                        failed += 1;
                    }
                }
            }
            SourceType::Local | SourceType::WellKnown | SourceType::NodeModules => {
                println!(
                    "skip {}: sourceType '{}' not installable by quay",
                    name,
                    source_type_label(entry.source_type)
                );
                skipped += 1;
            }
        }
    }

    if installed == 0 && skipped == 0 && failed == 0 {
        println!("{} up to date", lock::LOCKFILE_NAME);
    } else if failed == 0 {
        println!("synced: {} installed, {} skipped", installed, skipped);
    } else {
        println!(
            "synced: {} installed, {} skipped, {} failed",
            installed, skipped, failed
        );
        return Err(format!("sync incomplete: {failed} skill(s) failed to install").into());
    }
    Ok(())
}
