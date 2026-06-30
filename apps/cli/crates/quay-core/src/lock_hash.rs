//! Replica of vercel-labs/skills' `computeSkillFolderHash` (their
//! `src/local-lock.ts`).
//!
//! Collect every file under the skill directory recursively (skipping any
//! `.git` and `node_modules` subdirectory), sort by forward-slash relative
//! path, then `sha256( relpath_bytes ++ content_bytes )` for each in order.
//! The result is a bare lowercase hex digest (no `sha256:` prefix). Verified
//! byte-identical to vercel for ASCII-lowercase paths (see the test).
//!
//! Two deliberate divergences from vercel, each matters only at the edges:
//! - **Sort collation.** We sort by Rust byte order (`str::cmp`); vercel uses
//!   JS `localeCompare`. These agree for the lowercase-ASCII paths skills use
//!   in practice but can differ for mixed-case / non-ASCII names, which would
//!   change the digest. (JS default `localeCompare` is itself locale-dependent,
//!   so an exact match isn't well-defined without bundling ICU.)
//! - **Symlinks.** We skip them (see below); vercel does not special-case them.
//!   A skill that hashes in symlinked entries will differ.
//!
//! Neither tool normalizes line endings, so both share the Windows/Linux CRLF
//! hash difference (vercel issue #781).

use crate::error::{QuayError, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn folder_hash(skill_dir: &Path) -> Result<String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect(skill_dir, skill_dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, content) in &files {
        hasher.update(rel.as_bytes());
        hasher.update(content);
    }
    Ok(hex(&hasher.finalize()))
}

fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|source| QuayError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| QuayError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let name = entry.file_name();
        // Use the dir entry's own type (does not follow symlinks). Skipping
        // symlinks prevents cyclic-symlink recursion and out-of-tree content
        // leaking into the digest, and matches vercel's Dirent-based traversal.
        let file_type = entry.file_type().map_err(|source| QuayError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if name == ".git" || name == "node_modules" {
                continue;
            }
            collect(base, &path, out)?;
        } else if file_type.is_file() {
            let content = std::fs::read(&path).map_err(|source| QuayError::Io {
                path: path.display().to_string(),
                source,
            })?;
            let rel = path
                .strip_prefix(base)
                .expect("path is under base")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, content));
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;

    #[test]
    fn matches_reference_algorithm() {
        let dir = assert_fs::TempDir::new().unwrap();
        dir.child("SKILL.md").write_str("hello\n").unwrap();
        dir.child("sub/extra.txt").write_str("world\n").unwrap();
        let got = folder_hash(dir.path()).unwrap();
        assert_eq!(
            got,
            "c682c2977f5a777a060b479101a5b400fe8eb925f1f2be0a276a8a757bccf3aa"
        );
    }

    #[test]
    fn skips_git_and_node_modules() {
        let dir = assert_fs::TempDir::new().unwrap();
        dir.child("SKILL.md").write_str("hello\n").unwrap();
        dir.child("sub/extra.txt").write_str("world\n").unwrap();
        dir.child(".git/config").write_str("junk\n").unwrap();
        dir.child("node_modules/x/index.js")
            .write_str("junk\n")
            .unwrap();
        let got = folder_hash(dir.path()).unwrap();
        assert_eq!(
            got,
            "c682c2977f5a777a060b479101a5b400fe8eb925f1f2be0a276a8a757bccf3aa"
        );
    }

    #[test]
    fn order_independent_of_filesystem_order() {
        let dir = assert_fs::TempDir::new().unwrap();
        dir.child("sub/extra.txt").write_str("world\n").unwrap();
        dir.child("SKILL.md").write_str("hello\n").unwrap();
        let got = folder_hash(dir.path()).unwrap();
        assert_eq!(
            got,
            "c682c2977f5a777a060b479101a5b400fe8eb925f1f2be0a276a8a757bccf3aa"
        );
    }
}
