//! Enumerate the files that make up a skill package.
//!
//! A skill is the directory `.agents/skills/<name>/` (locally) or
//! `skills/<name>/` (on a hub). It always contains `SKILL.md` and may contain
//! arbitrary subfolders (`agents/`, `scripts/`, `resources/`, …). This module
//! is the single source of truth for *which* files belong to a package, so the
//! push copy step and the registry `files` list can never disagree.

use crate::error::{QuayError, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Content hash over exactly the files quay pushes — the [`collect_skill_files`]
/// set (dotfiles, dotdirs and symlinks excluded). A hub copy and a local
/// install of the same skill therefore produce the SAME digest, which
/// `outdated` relies on to tell "changed" from "unchanged" for hand-written
/// skills.
///
/// This is deliberately DISTINCT from [`crate::lock_hash::folder_hash`], which
/// hashes the whole folder (including dotfiles) for `skills-lock.json` drift.
/// Do not conflate them: `folder_hash` answers "did anything on disk change?";
/// this answers "does the local install match what the hub published?" — and
/// only the pushed file set can ever be on the hub.
pub fn pushable_content_hash(skill_dir: &Path) -> Result<String> {
    let rels = collect_skill_files(skill_dir)?;
    let mut hasher = Sha256::new();
    for rel in &rels {
        let full = skill_dir.join(rel);
        let content = std::fs::read(&full).map_err(|source| QuayError::Io {
            path: full.display().to_string(),
            source,
        })?;
        hasher.update(rel.as_bytes());
        hasher.update(&content);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Walk `skill_dir` depth-first and return the relative paths (POSIX `/`
/// separators) of every regular file to include in the package.
///
/// - `SKILL.md` is sorted first; the remaining paths follow in sorted order.
/// - Entries whose file name begins with `.` are skipped (dotfiles/dotdirs,
///   e.g. `.git`, `.DS_Store`).
/// - Symlinks (file or directory) are skipped, so a link cannot pull files
///   from outside the skill directory into a push/PR.
///
/// Returns [`QuayError::Io`] (with path context) if a directory cannot be read.
pub fn collect_skill_files(skill_dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk(skill_dir, skill_dir, &mut out)?;
    out.sort();
    // Hoist SKILL.md to the front if present (it sorts first already because
    // uppercase precedes lowercase, but make the contract explicit).
    if let Some(pos) = out.iter().position(|p| p == "SKILL.md") {
        let s = out.remove(pos);
        out.insert(0, s);
    }
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|source| QuayError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| QuayError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| QuayError::Io {
            path: entry.path().display().to_string(),
            source,
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk(root, &path, out)?;
        } else if file_type.is_file() {
            // strip_prefix(root) cannot fail: `path` was built by joining onto
            // a descendant of `root`.
            let rel = path.strip_prefix(root).expect("path is under root");
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel_str);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn touch(dir: &Path, rel: &str) {
        let full = dir.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, "x").unwrap();
    }

    #[test]
    fn flat_skill_lists_only_skill_md() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "SKILL.md");
        assert_eq!(
            collect_skill_files(tmp.path()).unwrap(),
            vec!["SKILL.md".to_string()]
        );
    }

    #[test]
    fn nested_skill_lists_nested_paths_skill_md_first() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "SKILL.md");
        touch(tmp.path(), "scripts/sync.mjs");
        touch(tmp.path(), "agents/openai.yaml");
        assert_eq!(
            collect_skill_files(tmp.path()).unwrap(),
            vec![
                "SKILL.md".to_string(),
                "agents/openai.yaml".to_string(),
                "scripts/sync.mjs".to_string(),
            ]
        );
    }

    #[test]
    fn skips_dotfiles_and_dotdirs() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "SKILL.md");
        touch(tmp.path(), ".DS_Store");
        touch(tmp.path(), ".git/config");
        assert_eq!(
            collect_skill_files(tmp.path()).unwrap(),
            vec!["SKILL.md".to_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "SKILL.md");
        // Place the target file in a separate temp dir so it is truly outside
        // the skill directory and is not collected as a regular file.
        let other = TempDir::new().unwrap();
        let outside = other.path().join("outside.txt");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, tmp.path().join("link.txt")).unwrap();
        assert_eq!(
            collect_skill_files(tmp.path()).unwrap(),
            vec!["SKILL.md".to_string()]
        );
    }

    #[test]
    fn unreadable_dir_errors() {
        let missing = TempDir::new().unwrap().path().join("does-not-exist");
        assert!(collect_skill_files(&missing).is_err());
    }

    #[test]
    fn pushable_hash_ignores_dotfiles_but_tracks_pushed_files() {
        // Baseline: SKILL.md + one sibling.
        let a = TempDir::new().unwrap();
        touch(a.path(), "SKILL.md");
        touch(a.path(), "scripts/run.sh");
        let base = pushable_content_hash(a.path()).unwrap();

        // Adding a dotfile must NOT change the hash (dotfiles are never pushed,
        // so the hub copy can't contain them — this is the C1 round-trip fix).
        let b = TempDir::new().unwrap();
        touch(b.path(), "SKILL.md");
        touch(b.path(), "scripts/run.sh");
        touch(b.path(), ".DS_Store");
        touch(b.path(), ".github/workflows/ci.yml");
        assert_eq!(pushable_content_hash(b.path()).unwrap(), base);

        // Editing a genuinely-pushed sibling MUST change the hash.
        let c = TempDir::new().unwrap();
        touch(c.path(), "SKILL.md");
        touch(c.path(), "scripts/run.sh"); // creates dir + "x"
        std::fs::write(c.path().join("scripts/run.sh"), "different").unwrap();
        assert_ne!(pushable_content_hash(c.path()).unwrap(), base);
    }
}
