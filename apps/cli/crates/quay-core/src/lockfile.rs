use crate::error::{QuayError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

const LOCKFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub lockfile_version: u32,
    #[serde(default)]
    pub remotes: BTreeMap<String, LockedRemote>,
    #[serde(default)]
    pub skills: BTreeMap<String, LockedSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedRemote {
    pub url: String,
    pub registry_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedSkill {
    pub remote: String,
    pub version: String,
    pub sha: String,
    /// The hub-side path under which this skill lives, e.g. "skills/csv-parse" or
    /// "skills/backend/csv-parse". Used by `quay sync` to refetch files at the
    /// correct location regardless of whether the hub uses flat or nested layout.
    /// Defaulted to empty string for compatibility with lockfiles produced before
    /// this field existed; sync falls back to "skills/<skill_name>" in that case.
    #[serde(default)]
    pub path: String,
    pub files: Vec<LockedFile>,
    pub installed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedFile {
    pub path: String,
    pub sha256: String,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            lockfile_version: LOCKFILE_VERSION,
            remotes: BTreeMap::new(),
            skills: BTreeMap::new(),
        }
    }
}

impl Lockfile {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(|source| QuayError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let lock: Lockfile =
            serde_json::from_str(&text).map_err(|e| QuayError::InvalidLockfile {
                reason: e.to_string(),
            })?;
        if lock.lockfile_version != LOCKFILE_VERSION {
            return Err(QuayError::InvalidLockfile {
                reason: format!(
                    "unsupported lockfile_version {} (expected {})",
                    lock.lockfile_version, LOCKFILE_VERSION
                ),
            });
        }
        Ok(lock)
    }

    /// Returns the recorded sha256 of a skill's primary file (the entry whose
    /// path ends in `SKILL.md`), or `None` if the skill is not in the lockfile.
    pub fn skill_primary_sha(&self, name: &str) -> Option<&str> {
        let entry = self.skills.get(name)?;
        entry
            .files
            .iter()
            .find(|f| f.path.ends_with("SKILL.md"))
            .or_else(|| entry.files.first())
            .map(|f| f.sha256.as_str())
    }

    /// Returns the recorded `(remote, version)` pair for an installed skill, or
    /// `None` if the skill is not in the lockfile.
    pub fn skill_remote_version(&self, name: &str) -> Option<(&str, &str)> {
        self.skills
            .get(name)
            .map(|s| (s.remote.as_str(), s.version.as_str()))
    }

    /// Atomic write: serialize to a temp file in the same directory, then rename.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|source| QuayError::Io {
            path: parent.display().to_string(),
            source,
        })?;
        let json = serde_json::to_string_pretty(self).map_err(|e| QuayError::InvalidLockfile {
            reason: e.to_string(),
        })?;
        tmp.write_all(json.as_bytes())
            .map_err(|source| QuayError::Io {
                path: tmp.path().display().to_string(),
                source,
            })?;
        tmp.persist(path).map_err(|e| QuayError::Io {
            path: path.display().to_string(),
            source: e.error,
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;

    #[test]
    fn missing_file_yields_default() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.child("skills.lock.json");
        let lock = Lockfile::load_or_default(path.path()).unwrap();
        assert_eq!(lock, Lockfile::default());
    }

    #[test]
    fn round_trip() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.child("skills.lock.json");
        let lock = Lockfile {
            lockfile_version: 1,
            remotes: BTreeMap::from([(
                "h".into(),
                LockedRemote {
                    url: "https://x/y.git".into(),
                    registry_sha: "deadbeef".into(),
                },
            )]),
            skills: BTreeMap::from([(
                "csv-parse".into(),
                LockedSkill {
                    remote: "h".into(),
                    version: "1.2.0".into(),
                    sha: "abc".into(),
                    path: "skills/csv-parse".into(),
                    files: vec![LockedFile {
                        path: "SKILL.md".into(),
                        sha256: "0".repeat(64),
                    }],
                    installed_at: "2026-05-08T10:39:00Z".into(),
                },
            )]),
        };
        lock.save(path.path()).unwrap();
        let read = Lockfile::load_or_default(path.path()).unwrap();
        assert_eq!(read, lock);
    }

    #[test]
    fn rejects_unknown_version() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.child("skills.lock.json");
        std::fs::write(
            path.path(),
            r#"{"lockfile_version": 999, "remotes": {}, "skills": {}}"#,
        )
        .unwrap();
        let err = Lockfile::load_or_default(path.path()).unwrap_err();
        assert!(format!("{}", err).contains("999"));
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.child("nested/deeper/skills.lock.json");
        Lockfile::default().save(path.path()).unwrap();
        assert!(path.path().exists());
    }

    #[test]
    fn skill_primary_sha_finds_skill_md_entry() {
        let mut lock = Lockfile::default();
        lock.skills.insert(
            "foo".into(),
            LockedSkill {
                remote: "r".into(),
                version: "1.0".into(),
                sha: "deadbeef".into(),
                path: "skills/foo".into(),
                files: vec![
                    LockedFile {
                        path: "skills/foo/extra.md".into(),
                        sha256: "111".into(),
                    },
                    LockedFile {
                        path: "skills/foo/SKILL.md".into(),
                        sha256: "222".into(),
                    },
                ],
                installed_at: "2026-05-09T00:00:00Z".into(),
            },
        );
        assert_eq!(lock.skill_primary_sha("foo"), Some("222"));
        assert_eq!(lock.skill_primary_sha("missing"), None);
    }
}
