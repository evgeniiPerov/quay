//! User-level intent log: skills the user has pushed via quay (PR opened),
//! independent of whether the PR has merged.
//!
//! As of quay 0.2.x the log lives at `~/.config/quay/push-log.json` (same
//! directory as `config.toml`). Per-project `.quay/push-log.json` files are
//! no longer written; legacy files are migrated on first access.
//!
//! Losing the file is non-fatal — status falls back to `Local`.

use crate::error::{QuayError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Absolute path of the project root this push originated from.
    ///
    /// `None` for records migrated from legacy per-project logs when the
    /// project root could not be determined, or records created before quay
    /// 0.2.x. Such records are treated as matching any project (legacy
    /// behaviour) by [`PushLog::latest_for_project`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<PathBuf>,
}

impl PushLog {
    /// Return the path to the user-level push-log given the quay config directory.
    ///
    /// The config directory is typically `~/.config/quay/` (same dir as
    /// `config.toml`).
    pub fn user_level_path(config_dir: &Path) -> PathBuf {
        config_dir.join("push-log.json")
    }

    /// Legacy per-project path: `<project_root>/.quay/push-log.json`.
    fn legacy_path(project_root: &Path) -> PathBuf {
        project_root.join(".quay/push-log.json")
    }

    /// Load the user-level log from `config_dir`.
    ///
    /// If the user-level log does not yet exist but a legacy
    /// `<project_root>/.quay/push-log.json` does, the legacy entries are
    /// automatically migrated into the user-level log with `project_path` set
    /// to `project_root`.  After migration the user-level file is written to
    /// disk; the legacy file is left in place.
    ///
    /// When `project_root` is `None`, migration is skipped.
    pub fn load(config_dir: &Path, project_root: Option<&Path>) -> Result<PushLog> {
        let user_path = Self::user_level_path(config_dir);

        if user_path.exists() {
            return Self::read_from(&user_path);
        }

        // User-level log absent — check for a legacy per-project file.
        if let Some(root) = project_root {
            let legacy = Self::legacy_path(root);
            if legacy.exists() {
                let mut log = Self::read_from(&legacy)?;
                // Populate project_path for each migrated record.
                let abs_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
                for rec in &mut log.records {
                    if rec.project_path.is_none() {
                        rec.project_path = Some(abs_root.clone());
                    }
                }
                // Persist to user-level so subsequent reads are from there.
                Self::write_log(config_dir, &log)?;
                return Ok(log);
            }
        }

        Ok(PushLog::default())
    }

    /// Append a record to the user-level log and persist atomically.
    pub fn append(config_dir: &Path, record: PushRecord) -> Result<()> {
        let mut existing = if Self::user_level_path(config_dir).exists() {
            Self::load(config_dir, None)?
        } else {
            PushLog::default()
        };
        existing.records.push(record);
        Self::write_log(config_dir, &existing)
    }

    /// Most recent record for a given skill name in the given project, or `None`.
    ///
    /// Records whose `project_path` matches `project_root` are returned.
    /// Records with no `project_path` (legacy / migrated-without-root) are
    /// also returned for backward compatibility.
    pub fn latest_for_project(&self, name: &str, project_root: &Path) -> Option<&PushRecord> {
        let abs_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
        self.records.iter().rev().find(|r| {
            r.name == name
                && match &r.project_path {
                    None => true, // legacy record — match any project
                    Some(p) => p == &abs_root,
                }
        })
    }

    /// Most recent record for a given skill name, ignoring project path.
    ///
    /// Used by the global Dashboard "recent pushes" panel.
    pub fn latest_for(&self, name: &str) -> Option<&PushRecord> {
        self.records.iter().rev().find(|r| r.name == name)
    }

    // ── private helpers ──────────────────────────────────────────────────────

    fn read_from(path: &Path) -> Result<PushLog> {
        let text = std::fs::read_to_string(path).map_err(|e| QuayError::Io {
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

    fn write_log(config_dir: &Path, log: &PushLog) -> Result<()> {
        std::fs::create_dir_all(config_dir).map_err(|e| QuayError::Io {
            path: config_dir.display().to_string(),
            source: e,
        })?;
        let path = Self::user_level_path(config_dir);
        let body = serde_json::to_string_pretty(log).map_err(|e| QuayError::InvalidPushLog {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_record(name: &str, project: Option<&Path>) -> PushRecord {
        PushRecord {
            name: name.into(),
            remote: "hub".into(),
            branch: format!("quay/{name}-1.0"),
            pr_url: "https://example/pr/1".into(),
            pushed_at: "2026-05-09T18:30:00Z".into(),
            commit_sha: Some("aabbccdd".into()),
            // Canonicalized to match what pusher.rs writes in production. On
            // Windows canonicalize adds the \\?\ prefix and expands 8.3 names,
            // so a raw path would never match the lookup.
            project_path: project.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf())),
        }
    }

    // ── push_log_rw_user_level ───────────────────────────────────────────────

    #[test]
    fn push_log_rw_user_level() {
        let config_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();

        let rec = make_record("foo", Some(project_dir.path()));
        PushLog::append(config_dir.path(), rec.clone()).unwrap();

        // User-level file must exist.
        assert!(PushLog::user_level_path(config_dir.path()).exists());

        let log = PushLog::load(config_dir.path(), Some(project_dir.path())).unwrap();
        assert_eq!(log.records.len(), 1);
        assert_eq!(log.records[0].name, "foo");
    }

    // ── push_log_legacy_migration ─────────────────────────────────────────────

    #[test]
    fn push_log_legacy_migration() {
        let config_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();

        // Write a legacy per-project log (no project_path field).
        let legacy_dir = project_dir.path().join(".quay");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let body = r#"{
            "records": [{
                "name": "bar",
                "remote": "hub",
                "branch": "quay/bar-1.0",
                "pr_url": "https://example/pr/2",
                "pushed_at": "2026-05-08T10:00:00Z"
            }]
        }"#;
        std::fs::write(legacy_dir.join("push-log.json"), body).unwrap();

        // First read: no user-level log → migration happens.
        let log = PushLog::load(config_dir.path(), Some(project_dir.path())).unwrap();
        assert_eq!(log.records.len(), 1);
        assert_eq!(log.records[0].name, "bar");
        // project_path must be populated after migration.
        assert!(
            log.records[0].project_path.is_some(),
            "project_path must be set after migration"
        );

        // User-level file must now exist.
        assert!(PushLog::user_level_path(config_dir.path()).exists());

        // Second read: user-level log exists → reads from there, no re-migration.
        let log2 = PushLog::load(config_dir.path(), Some(project_dir.path())).unwrap();
        assert_eq!(log2.records.len(), 1);
    }

    // ── pushed_local_status_filters_by_project ───────────────────────────────

    #[test]
    fn pushed_local_status_filters_by_project() {
        let project_a = TempDir::new().unwrap();
        let project_b = TempDir::new().unwrap();

        let mut log = PushLog::default();
        log.records
            .push(make_record("skill", Some(project_a.path())));
        log.records
            .push(make_record("skill", Some(project_b.path())));

        // Project A sees its own record.
        let rec_a = log.latest_for_project("skill", project_a.path());
        assert!(rec_a.is_some());
        assert_eq!(
            rec_a.unwrap().project_path.as_deref(),
            project_a.path().canonicalize().ok().as_deref()
        );

        // Project B sees its own record.
        let rec_b = log.latest_for_project("skill", project_b.path());
        assert!(rec_b.is_some());
        assert_eq!(
            rec_b.unwrap().project_path.as_deref(),
            project_b.path().canonicalize().ok().as_deref()
        );
    }

    // ── legacy_record_matches_any_project ────────────────────────────────────

    #[test]
    fn legacy_record_matches_any_project() {
        let project = TempDir::new().unwrap();
        let mut log = PushLog::default();
        // Legacy record has no project_path.
        log.records.push(PushRecord {
            name: "baz".into(),
            remote: "hub".into(),
            branch: "quay/baz-1.0".into(),
            pr_url: "https://example/pr/3".into(),
            pushed_at: "2026-05-01T00:00:00Z".into(),
            commit_sha: None,
            project_path: None,
        });
        // Should match even though project_path is None.
        assert!(log.latest_for_project("baz", project.path()).is_some());
    }

    // ── backward-compat: old records without commit_sha load fine ────────────

    #[test]
    fn old_records_without_commit_sha_load_with_none() {
        let config_dir = TempDir::new().unwrap();
        let config_quay_dir = config_dir.path();
        std::fs::create_dir_all(config_quay_dir).unwrap();
        let body = r#"{
            "records": [{
                "name": "foo",
                "remote": "hub",
                "branch": "quay/foo-1.0",
                "pr_url": "https://example/pr/1",
                "pushed_at": "2026-05-09T18:30:00Z"
            }]
        }"#;
        std::fs::write(PushLog::user_level_path(config_quay_dir), body).unwrap();
        let log = PushLog::load(config_quay_dir, None).unwrap();
        assert_eq!(log.records[0].commit_sha, None);
    }

    // ── load missing file returns empty ──────────────────────────────────────

    #[test]
    fn load_missing_file_returns_empty() {
        let config_dir = TempDir::new().unwrap();
        let log = PushLog::load(config_dir.path(), None).unwrap();
        assert!(log.records.is_empty());
    }

    // ── append then load round-trips ─────────────────────────────────────────

    #[test]
    fn append_then_load_round_trips_one_record() {
        let config_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();

        let rec = PushRecord {
            name: "foo".into(),
            remote: "hub".into(),
            branch: "quay/foo-1.0".into(),
            pr_url: "https://example/pr/1".into(),
            pushed_at: "2026-05-09T18:30:00Z".into(),
            commit_sha: Some("aabbccdd".into()),
            project_path: Some(project_dir.path().to_path_buf()),
        };
        PushLog::append(config_dir.path(), rec.clone()).unwrap();
        let log = PushLog::load(config_dir.path(), Some(project_dir.path())).unwrap();
        assert_eq!(log.records, vec![rec]);
    }

    // ── latest_for_project returns most recent matching ───────────────────────

    #[test]
    fn latest_for_project_returns_most_recent() {
        let project = TempDir::new().unwrap();
        let earlier = make_record("foo", Some(project.path()));
        let later = PushRecord {
            pushed_at: "2026-05-09T12:00:00Z".into(),
            pr_url: "https://example/pr/2".into(),
            ..earlier.clone()
        };
        let mut log = PushLog::default();
        log.records.push(earlier);
        log.records.push(later.clone());
        let found = log.latest_for_project("foo", project.path()).unwrap();
        assert_eq!(found.pr_url, later.pr_url);
    }
}
