//! Enumerate the files that make up a skill package.
//!
//! A skill is the directory `.agents/skills/<name>/` (locally) or
//! `skills/<name>/` (on a hub). It always contains `SKILL.md` and may contain
//! arbitrary subfolders (`agents/`, `scripts/`, `resources/`, …). This module
//! is the single source of truth for *which* files belong to a package, so the
//! push copy step and the registry `files` list can never disagree.

use crate::error::{QuayError, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
    hash_with(skill_dir, false)
}

/// Same digest, but computed as if every UTF-8 text file used LF line endings.
///
/// git's default `core.autocrlf` on Windows rewrites line endings at checkout,
/// so a skill installed there differs byte-for-byte from the LF copy the hub
/// hashed — and a raw comparison reports drift for every skill on the platform.
/// Comparing against this digest as well absorbs that.
///
/// It deliberately does NOT replace [`pushable_content_hash`]: that digest is
/// what registry writers publish, and changing it would invalidate the
/// `content_hash` of every registry.json already in the wild. Non-UTF-8 files
/// are hashed raw, so a binary asset is unaffected.
///
/// Residual gap: this normalizes the *local* side only. A hub whose published
/// hash was computed from CRLF bytes still mismatches an LF checkout — fixing
/// that direction means normalizing at publish time, which is a registry
/// format migration.
pub fn pushable_content_hash_lf(skill_dir: &Path) -> Result<String> {
    hash_with(skill_dir, true)
}

fn hash_with(skill_dir: &Path, normalize_eol: bool) -> Result<String> {
    let mut files = BTreeMap::new();
    for rel in collect_skill_files(skill_dir)? {
        let full = skill_dir.join(&rel);
        let content = std::fs::read(&full).map_err(|source| QuayError::Io {
            path: full.display().to_string(),
            source,
        })?;
        let content = match normalize_eol {
            true => normalize_crlf(content),
            false => content,
        };
        files.insert(rel, content);
    }
    Ok(content_hash_of(&files))
}

/// [`pushable_content_hash`] over a file set already in memory.
///
/// The folder-level comparison in `reconcile::folder` holds a harbor tree as
/// bytes and has no directory to walk; hashing it through the same function
/// keeps the two sides comparable. (That caller normalizes line endings first,
/// so its digests are not the value a registry publishes — only equal-to-each-
/// other. A caller that passes raw bytes does reproduce the published digest.)
///
/// The caller owns the file set: keys must be skill-directory-relative POSIX
/// paths, with dotfiles and symlinks already excluded, exactly as
/// [`collect_skill_files`] returns them.
pub fn content_hash_of(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    // SKILL.md first, then the rest in sorted order — `collect_skill_files`
    // hoists it, so hashing a BTreeMap in plain key order would disagree for a
    // skill carrying a file that sorts ahead of it (e.g. `AGENTS.md`).
    let ordered = files
        .iter()
        .filter(|(rel, _)| rel.as_str() == "SKILL.md")
        .chain(files.iter().filter(|(rel, _)| rel.as_str() != "SKILL.md"));
    for (rel, content) in ordered {
        // Length-prefix each field so distinct (path, content) splits can never
        // hash the same (e.g. "a"+"bc" vs "ab"+"c").
        hasher.update((rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update((content.len() as u64).to_le_bytes());
        hasher.update(content);
    }
    hex::encode(hasher.finalize())
}

/// CRLF -> LF for valid UTF-8; binary content is returned untouched.
///
/// Public because `reconcile::folder` needs the same treatment: it compares a
/// harbor tree against a working copy, and on Windows the working copy holds
/// CRLF while the harbor's blobs hold LF.
pub fn normalize_crlf(content: Vec<u8>) -> Vec<u8> {
    match String::from_utf8(content) {
        Ok(s) if s.contains("\r\n") => s.replace("\r\n", "\n").into_bytes(),
        Ok(s) => s.into_bytes(),
        Err(e) => e.into_bytes(),
    }
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
    fn lf_hash_collapses_crlf_but_leaves_binary_bytes_alone() {
        let lf = TempDir::new().unwrap();
        fs::write(lf.path().join("SKILL.md"), "a\nb\n").unwrap();
        fs::write(lf.path().join("logo.png"), [0xff, 0x0d, 0x0a, 0xfe]).unwrap();

        let crlf = TempDir::new().unwrap();
        fs::write(crlf.path().join("SKILL.md"), "a\r\nb\r\n").unwrap();
        // Same bytes: a 0d0a inside binary content must NOT be rewritten, or the
        // digest would depend on where a byte pair happened to land.
        fs::write(crlf.path().join("logo.png"), [0xff, 0x0d, 0x0a, 0xfe]).unwrap();

        assert_ne!(
            pushable_content_hash(lf.path()).unwrap(),
            pushable_content_hash(crlf.path()).unwrap(),
            "the raw digest stays byte-exact — it is what registries publish"
        );
        assert_eq!(
            pushable_content_hash_lf(lf.path()).unwrap(),
            pushable_content_hash_lf(crlf.path()).unwrap(),
            "the LF digest sees the two checkouts as the same skill"
        );

        // Binary content still has to reach the digest — normalization must not
        // quietly drop the bytes it cannot decode.
        let other = TempDir::new().unwrap();
        fs::write(other.path().join("SKILL.md"), "a\nb\n").unwrap();
        fs::write(other.path().join("logo.png"), [0xff, 0x0d, 0x0a, 0x00]).unwrap();
        assert_ne!(
            pushable_content_hash_lf(lf.path()).unwrap(),
            pushable_content_hash_lf(other.path()).unwrap(),
            "an edited binary asset must change the LF digest"
        );
    }

    #[test]
    fn in_memory_hash_matches_the_on_disk_one() {
        // The folder report hashes a harbor tree it holds in memory and compares
        // it against a local directory. The two must be the same digest, or the
        // comparison is meaningless — and matching `pushable_content_hash` also
        // keeps it comparable to the `content_hash` registries publish.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("SKILL.md"), "body").unwrap();
        // Sorts BEFORE "SKILL.md" bytewise, so this catches an implementation
        // that sorts naively instead of hoisting SKILL.md first.
        fs::write(dir.path().join("AGENTS.md"), "agents").unwrap();
        fs::create_dir_all(dir.path().join("scripts")).unwrap();
        fs::write(dir.path().join("scripts/run.sh"), "echo").unwrap();

        let in_memory = BTreeMap::from([
            ("SKILL.md".to_string(), b"body".to_vec()),
            ("AGENTS.md".to_string(), b"agents".to_vec()),
            ("scripts/run.sh".to_string(), b"echo".to_vec()),
        ]);
        assert_eq!(
            content_hash_of(&in_memory),
            pushable_content_hash(dir.path()).unwrap()
        );
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
