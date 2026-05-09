//! Local-only intent log: skills the user has pushed via quay (PR opened),
//! independent of whether the PR has merged.
//!
//! Persisted at `<project_root>/.quay/push-log.json`. Gitignored —
//! losing the file is non-fatal (status falls back to `Local`).

use crate::error::{QuayError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The full push-log as loaded from disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushLog {
    #[serde(default)]
    pub records: Vec<PushRecord>,
}

/// One push event recorded locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRecord {
    pub name: String,
    pub remote: String,
    pub branch: String,
    /// Empty string for direct-mode pushes.
    pub pr_url: String,
    /// RFC 3339 timestamp.
    pub pushed_at: String,
    /// New in Plan 9. `None` for old records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}

impl PushLog {
    /// Load the log from disk; an absent file deserialises to an empty log.
    pub fn load(project_root: &Path) -> Result<PushLog> {
        let path = project_root.join(".quay/push-log.json");
        if !path.exists() {
            return Ok(PushLog::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| QuayError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        if text.trim().is_empty() {
            return Ok(PushLog::default());
        }
        serde_json::from_str(&text).map_err(|e| QuayError::InvalidPushLog {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
    }

    /// Append a record and persist to disk atomically (write-to-tmp + rename).
    pub fn append(project_root: &Path, record: PushRecord) -> Result<()> {
        let dir = project_root.join(".quay");
        std::fs::create_dir_all(&dir).map_err(|e| QuayError::Io {
            path: dir.display().to_string(),
            source: e,
        })?;
        let path = dir.join("push-log.json");
        let mut existing = if path.exists() {
            PushLog::load(project_root)?
        } else {
            PushLog::default()
        };
        existing.records.push(record);
        let body =
            serde_json::to_string_pretty(&existing).map_err(|e| QuayError::InvalidPushLog {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &body).map_err(|e| QuayError::Io {
            path: tmp.display().to_string(),
            source: e,
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| QuayError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Ok(())
    }

    /// Most recent record for a given skill name, or `None`.
    pub fn latest_for(&self, name: &str) -> Option<&PushRecord> {
        self.records.iter().rev().find(|r| r.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_then_load_round_trips_one_record() {
        let tmp = TempDir::new().unwrap();
        let rec = PushRecord {
            name: "foo".into(),
            remote: "hub".into(),
            branch: "quay/foo-1.0".into(),
            pr_url: "https://example/pr/1".into(),
            pushed_at: "2026-05-09T18:30:00Z".into(),
            commit_sha: Some("aabbccdd".into()),
        };
        PushLog::append(tmp.path(), rec.clone()).unwrap();
        let log = PushLog::load(tmp.path()).unwrap();
        assert_eq!(log.records, vec![rec]);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let log = PushLog::load(tmp.path()).unwrap();
        assert!(log.records.is_empty());
    }

    #[test]
    fn old_records_without_commit_sha_load_with_none() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".quay");
        std::fs::create_dir_all(&dir).unwrap();
        let body = r#"{
            "records": [{
                "name": "foo",
                "remote": "hub",
                "branch": "quay/foo-1.0",
                "pr_url": "https://example/pr/1",
                "pushed_at": "2026-05-09T18:30:00Z"
            }]
        }"#;
        std::fs::write(dir.join("push-log.json"), body).unwrap();
        let log = PushLog::load(tmp.path()).unwrap();
        assert_eq!(log.records[0].commit_sha, None);
    }

    #[test]
    fn latest_for_returns_most_recent() {
        let tmp = TempDir::new().unwrap();
        let earlier = PushRecord {
            name: "foo".into(),
            remote: "hub".into(),
            branch: "quay/foo-1.0".into(),
            pr_url: "https://example/pr/1".into(),
            pushed_at: "2026-05-08T12:00:00Z".into(),
            commit_sha: None,
        };
        let later = PushRecord {
            pushed_at: "2026-05-09T12:00:00Z".into(),
            pr_url: "https://example/pr/2".into(),
            ..earlier.clone()
        };
        PushLog::append(tmp.path(), earlier).unwrap();
        PushLog::append(tmp.path(), later.clone()).unwrap();
        let log = PushLog::load(tmp.path()).unwrap();
        assert_eq!(log.latest_for("foo"), Some(&later));
    }
}
