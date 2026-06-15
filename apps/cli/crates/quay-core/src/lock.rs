//! `skills-lock.json` — the vercel-labs/skills lockfile, read and written
//! byte-compatibly so the `skills` npm CLI and quay share one file.
//!
//! Adopting this lockfile reverses the Plan 10 "no lockfile" stance for the
//! purpose of recording *what is installed and from where*. See
//! `docs/superpowers/specs/2026-06-09-skills-lock-interop-design.md`.

use crate::error::{QuayError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SUPPORTED_VERSION: u32 = 1;
pub const LOCKFILE_NAME: &str = "skills-lock.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsLock {
    pub version: u32,
    pub skills: BTreeMap<String, LockEntry>,
}

impl SkillsLock {
    pub fn empty() -> Self {
        Self { version: SUPPORTED_VERSION, skills: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockEntry {
    pub source: String,
    pub source_type: SourceType,
    pub skill_path: String,
    pub computed_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    Github,
    Git,
    Local,
    WellKnown,
    NodeModules,
}

/// Serialize with two-space indent + trailing newline, matching vercel's style.
pub fn to_pretty_json(lock: &SkillsLock) -> String {
    let mut s = serde_json::to_string_pretty(lock).expect("SkillsLock serializes");
    s.push('\n');
    s
}

fn lock_path(project_root: &Path) -> PathBuf {
    project_root.join(LOCKFILE_NAME)
}

/// Read the lockfile at `project_root`. `Ok(None)` when absent.
pub fn read(project_root: &Path) -> Result<Option<SkillsLock>> {
    let path = lock_path(project_root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(QuayError::Io { path: path.display().to_string(), source }),
    };
    let lock: SkillsLock = serde_json::from_str(&raw)
        .map_err(|e| QuayError::InvalidLockfile { reason: format!("{}: {e}", path.display()) })?;
    if lock.version > SUPPORTED_VERSION {
        return Err(QuayError::InvalidLockfile {
            reason: format!(
                "lockfile version {} is newer than supported version {}; upgrade quay",
                lock.version, SUPPORTED_VERSION
            ),
        });
    }
    Ok(Some(lock))
}

/// Map a git remote URL to a lockfile `(source, sourceType)`.
///
/// github.com URLs collapse to `owner/repo` with `SourceType::Github`; every
/// other host is preserved verbatim as a `SourceType::Git` clone URL — which is
/// how quay's gitlab / bitbucket / azure / GHE remotes are represented.
pub fn source_from_url(url: &str) -> (String, SourceType) {
    if let Some(owner_repo) = github_owner_repo(url) {
        (owner_repo, SourceType::Github)
    } else {
        (url.to_string(), SourceType::Git)
    }
}

fn github_owner_repo(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if !lower.contains("github.com") {
        return None;
    }
    let after_host = url.split_once("github.com").map(|(_, rest)| rest.trim_start_matches([':', '/']))?;
    let trimmed = after_host.trim_end_matches('/');
    let path = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(format!("{owner}/{repo}"))
}

/// Write the lockfile atomically (temp file in the same dir, then rename).
pub fn write_atomic(project_root: &Path, lock: &SkillsLock) -> Result<()> {
    let path = lock_path(project_root);
    let tmp = path.with_extension("json.tmp");
    let body = to_pretty_json(lock);
    std::fs::write(&tmp, body.as_bytes())
        .map_err(|source| QuayError::Io { path: tmp.display().to_string(), source })?;
    if let Err(source) = std::fs::rename(&tmp, &path) {
        // Don't leave the temp file behind if the rename fails.
        let _ = std::fs::remove_file(&tmp);
        return Err(QuayError::Io { path: path.display().to_string(), source });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "version": 1,
  "skills": {
    "tdd": {
      "source": "mattpocock/skills",
      "sourceType": "github",
      "skillPath": "skills/engineering/tdd/SKILL.md",
      "computedHash": "15a7b5e36383ebadb2dec5e586679e55e9663d292da418926b8da6fc0ef27d84"
    }
  }
}"#;

    #[test]
    fn parses_vercel_lockfile() {
        let lock: SkillsLock = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(lock.version, 1);
        let e = &lock.skills["tdd"];
        assert_eq!(e.source, "mattpocock/skills");
        assert_eq!(e.source_type, SourceType::Github);
        assert_eq!(e.skill_path, "skills/engineering/tdd/SKILL.md");
        assert_eq!(e.computed_hash, "15a7b5e36383ebadb2dec5e586679e55e9663d292da418926b8da6fc0ef27d84");
    }

    #[test]
    fn round_trips_field_names_and_source_type_strings() {
        let lock: SkillsLock = serde_json::from_str(SAMPLE).unwrap();
        let out = to_pretty_json(&lock);
        assert!(out.contains("\"sourceType\": \"github\""));
        assert!(out.contains("\"skillPath\":"));
        assert!(out.contains("\"computedHash\":"));
        // vercel on-disk style: two-space indent + trailing newline.
        assert!(out.starts_with("{\n  \""), "expected 2-space indent: {out}");
        assert!(out.ends_with("}\n"), "expected trailing newline");
        let reparsed: SkillsLock = serde_json::from_str(&out).unwrap();
        assert_eq!(reparsed, lock);
    }

    #[test]
    fn source_type_renders_kebab_case() {
        assert_eq!(serde_json::to_string(&SourceType::WellKnown).unwrap(), "\"well-known\"");
        assert_eq!(serde_json::to_string(&SourceType::NodeModules).unwrap(), "\"node-modules\"");
    }

    #[test]
    fn read_missing_lockfile_returns_none() {
        let dir = assert_fs::TempDir::new().unwrap();
        assert!(read(dir.path()).unwrap().is_none());
    }

    #[test]
    fn write_then_read_is_identity() {
        let dir = assert_fs::TempDir::new().unwrap();
        let lock: SkillsLock = serde_json::from_str(SAMPLE).unwrap();
        write_atomic(dir.path(), &lock).unwrap();
        let back = read(dir.path()).unwrap().unwrap();
        assert_eq!(back, lock);
    }

    #[test]
    fn rejects_future_version() {
        let dir = assert_fs::TempDir::new().unwrap();
        std::fs::write(dir.path().join("skills-lock.json"), r#"{"version":99,"skills":{}}"#).unwrap();
        let err = read(dir.path()).unwrap_err();
        assert!(matches!(err, crate::error::QuayError::InvalidLockfile { .. }));
    }

    #[test]
    fn github_url_maps_to_owner_repo() {
        let (src, ty) = source_from_url("https://github.com/mattpocock/skills.git");
        assert_eq!(src, "mattpocock/skills");
        assert_eq!(ty, SourceType::Github);
    }

    #[test]
    fn github_ssh_maps_to_owner_repo() {
        let (src, ty) = source_from_url("git@github.com:acme/hub.git");
        assert_eq!(src, "acme/hub");
        assert_eq!(ty, SourceType::Github);
    }

    #[test]
    fn non_github_maps_to_git_full_url() {
        let (src, ty) = source_from_url("https://gitlab.com/acme/hub.git");
        assert_eq!(src, "https://gitlab.com/acme/hub.git");
        assert_eq!(ty, SourceType::Git);
    }
}
