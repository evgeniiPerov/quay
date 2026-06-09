//! `quay lock` — generate / check / heal / sync `skills-lock.json`.

use quay_core::lock::{self, LockEntry, SkillsLock, SourceType};
use quay_core::lock_hash::folder_hash;
use quay_core::push_log::PushLog;
use quay_core::scanner::{scan_local, LocalSkill};
use std::path::Path;

/// Build a `SkillsLock` from the skills currently on disk.
///
/// Origin is unknown from a plain scan, so entries are recorded as
/// `SourceType::Local` with the repo-relative `SKILL.md` path. Later tasks
/// record real github/git provenance when quay performs the install.
fn build_lock_from_disk(project_root: &Path, skills: &[LocalSkill]) -> SkillsLock {
    let mut lock = SkillsLock::empty();
    for s in skills {
        let skill_md = s.canonical_path();
        let folder = skill_md.parent().unwrap_or(skill_md);
        // A failed hash would make `--check` report this skill as perpetually
        // drifted, so surface the cause instead of silently writing an empty hash.
        let hash = folder_hash(folder).unwrap_or_else(|e| {
            eprintln!("warn: could not hash {}: {e}", folder.display());
            String::new()
        });
        let rel = skill_md
            .strip_prefix(project_root)
            .unwrap_or(skill_md)
            .to_string_lossy()
            .replace('\\', "/");
        lock.skills.insert(
            s.meta.name.clone(),
            LockEntry {
                // source == skill_path here: a plain scan can't know the real
                // origin, so we default both to the local path. Install-aware
                // tasks overwrite `source` with the github/git provenance.
                source: rel.clone(),
                source_type: SourceType::Local,
                skill_path: rel,
                computed_hash: hash,
            },
        );
    }
    lock
}

pub fn run(
    project_root: &Path,
    check: bool,
    heal: bool,
    sync: bool,
    online: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = crate::config_io::default_config_dir();
    let push_log = PushLog::load(
        config_dir.as_deref().unwrap_or(project_root),
        Some(project_root),
    )
    .unwrap_or_default();
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
        let lock = build_lock_from_disk(project_root, &skills);
        lock::write_atomic(project_root, &lock)?;
        println!("healed {} ({} skills)", lock::LOCKFILE_NAME, lock.skills.len());
    } else {
        let lock = build_lock_from_disk(project_root, &skills);
        lock::write_atomic(project_root, &lock)?;
        println!("wrote {} ({} skills)", lock::LOCKFILE_NAME, lock.skills.len());
    }
    Ok(())
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
fn compute_drift(project_root: &Path, skills: &[LocalSkill]) -> Result<Vec<Drift>, Box<dyn std::error::Error>> {
    let lock = lock::read(project_root)?.unwrap_or_else(SkillsLock::empty);
    let mut drift = Vec::new();
    let mut on_disk = std::collections::BTreeSet::new();
    for s in skills {
        on_disk.insert(s.meta.name.clone());
        match lock.skills.get(&s.meta.name) {
            None => drift.push(Drift::Untracked(s.meta.name.clone())),
            Some(entry) => {
                let folder = s.canonical_path().parent().unwrap_or_else(|| s.canonical_path());
                let hash = folder_hash(folder).unwrap_or_default();
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

fn check_impl(project_root: &Path, skills: &[LocalSkill], _online: bool) -> Result<(), Box<dyn std::error::Error>> {
    let drift = compute_drift(project_root, skills)?;
    if drift.is_empty() {
        println!("{} is in sync", lock::LOCKFILE_NAME);
        return Ok(());
    }
    for d in &drift {
        eprintln!("{}", d.line());
    }
    eprintln!("{} drift finding(s); run `quay lock --heal`", drift.len());
    std::process::exit(1);
}

fn sync_impl(_project_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
