//! On-the-fly outdated detection: compare local-file SHA-256 against the
//! remote hub's registry entry.
//!
//! A `skills-lock.json` now records what is installed and from where (see the
//! `lock` module). Version comparison here still uses the remote `registry.json`
//! `sha` and `version` as the upgrade signal — the hub `sha` is a git-object SHA
//! (not a file-content SHA), so `version` is the primary signal and sha mismatch
//! is an informational column. The lockfile contributes a `locked` flag per row
//! and offline content-hash drift detection via `quay lock --check`.

use crate::config::Config;
use crate::error::Result;
use crate::fetcher::RegistryFetcher;
use crate::scanner::{scan_local, LocalSkill};
use semver::Version;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::Path;

/// One row of `quay outdated` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutdatedEntry {
    pub name: String,
    pub remote: String,
    /// Version found in `registry.json`.
    pub available: String,
    /// SHA-256 of the local canonical `SKILL.md`.
    pub local_sha: String,
    /// SHA from `registry.json` (`entry.sha`).
    pub remote_sha: String,
    /// True when `available` is a higher semver than the locally parsed version.
    pub upgrade_available: bool,
    /// True when this skill is recorded in `skills-lock.json`.
    pub locked: bool,
}

/// Compare every locally-found skill against every configured remote's
/// registry. Returns one entry per (skill, remote) pair where the remote
/// publishes the skill.
///
/// Skills whose local version or remote version cannot be parsed as semver
/// are reported with `upgrade_available = false` (no panic).
///
/// Skills that are not published on any remote are omitted.
///
/// `config_dir` is the user-level quay config directory (e.g.
/// `~/.config/quay/`). When `None`, the push-log is treated as empty and
/// push status is ignored for the outdated comparison.
pub fn outdated_for_local<R: RegistryFetcher>(
    project_root: &Path,
    config_dir: Option<&Path>,
    config: &Config,
    fetcher: &R,
) -> Result<Vec<OutdatedEntry>> {
    let push_log = config_dir
        .map(|d| crate::push_log::PushLog::load(d, Some(project_root)).unwrap_or_default())
        .unwrap_or_default();
    let local_skills = scan_local(project_root, &push_log);
    let locked_names: BTreeSet<String> = crate::lock::read(project_root)
        .ok()
        .flatten()
        .map(|l| l.skills.keys().cloned().collect())
        .unwrap_or_default();
    outdated_for_skills(&local_skills, config, fetcher, &locked_names)
}

/// Core comparison logic — exposed separately so callers that already have a
/// `Vec<LocalSkill>` can avoid the extra filesystem scan.
pub fn outdated_for_skills<R: RegistryFetcher>(
    skills: &[LocalSkill],
    config: &Config,
    fetcher: &R,
    locked_names: &BTreeSet<String>,
) -> Result<Vec<OutdatedEntry>> {
    let mut rows = Vec::new();
    let mut registry_cache: std::collections::BTreeMap<String, crate::registry::Registry> =
        std::collections::BTreeMap::new();

    for skill in skills {
        let local_sha = skill.canonical_sha256().to_string();
        let local_version = skill.meta.version.clone();

        for (remote_name, remote_cfg) in &config.remotes {
            let registry = match registry_cache.get(remote_name) {
                Some(r) => r.clone(),
                None => {
                    let r = match remote_cfg.direct_branch.as_deref() {
                        Some(b) => fetcher.fetch_at(&remote_cfg.url, b)?,
                        None => fetcher.fetch(&remote_cfg.url)?,
                    };
                    registry_cache.insert(remote_name.clone(), r.clone());
                    r
                }
            };
            let Some(entry) = registry.entry(&skill.meta.name) else {
                continue;
            };

            let upgrade_available = match (
                Version::parse(&entry.version),
                Version::parse(&local_version),
            ) {
                (Ok(av), Ok(loc)) => av.cmp(&loc) == Ordering::Greater,
                _ => false,
            };

            rows.push(OutdatedEntry {
                name: skill.meta.name.clone(),
                remote: remote_name.clone(),
                available: entry.version.clone(),
                local_sha: local_sha.clone(),
                remote_sha: entry.sha.clone(),
                upgrade_available,
                locked: locked_names.contains(&skill.meta.name),
            });
        }
    }

    rows.sort_by(|a, b| {
        (a.remote.as_str(), a.name.as_str()).cmp(&(b.remote.as_str(), b.name.as_str()))
    });
    Ok(rows)
}

/// Legacy compatibility alias — kept temporarily while CLI commands are updated.
/// Use [`outdated_for_local`] in new code.
pub fn outdated<R: RegistryFetcher>(
    project_root: &Path,
    config: &Config,
    fetcher: &R,
) -> Result<Vec<OutdatedEntry>> {
    outdated_for_local(project_root, None, config, fetcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfig;
    use crate::registry::{Registry, RegistryEntry};
    use crate::scanner::{LocalLocation, LocalSkill, ScanStatus, SkillFormat, SkillMeta};
    use std::collections::BTreeMap;

    struct FakeRegistry(Registry);
    impl RegistryFetcher for FakeRegistry {
        fn fetch(&self, _url: &str) -> Result<Registry> {
            Ok(self.0.clone())
        }
    }

    fn make_config() -> Config {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            RemoteConfig {
                url: "https://github.com/foo/bar.git".into(),
                default: true,
                provider: None,
                push_mode: crate::config::PushMode::default(),
                direct_branch: None,
            },
        );
        cfg
    }

    fn make_registry(version: &str) -> Registry {
        Registry {
            hub: "h".into(),
            generated_at: "2026-05-10T00:00:00Z".into(),
            schema_version: 1,
            skills: BTreeMap::from([(
                "csv-parse".into(),
                RegistryEntry {
                    version: version.into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: format!("remote-sha-{}", version),
                    files: vec!["SKILL.md".into()],
                    source_format: SkillFormat::Frontmatter,
                },
            )]),
        }
    }

    fn make_local_skill(version: &str, sha: &str) -> LocalSkill {
        LocalSkill {
            meta: SkillMeta {
                name: "csv-parse".into(),
                description: "Parse CSV.".into(),
                version: version.into(),
                tags: vec![],
                format: SkillFormat::Frontmatter,
            },
            locations: vec![LocalLocation {
                root: crate::config::MirrorRoot::Agents,
                path: std::path::PathBuf::from("/tmp/csv-parse/SKILL.md"),
                sha256: sha.into(),
            }],
            status: ScanStatus::Local,
        }
    }

    #[test]
    fn upgrade_available_when_registry_is_newer() {
        let cfg = make_config();
        let skills = vec![make_local_skill("1.0.0", "local-sha")];
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated_for_skills(&skills, &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].upgrade_available);
        assert_eq!(rows[0].available, "1.2.0");
    }

    #[test]
    fn no_upgrade_when_registry_matches() {
        let cfg = make_config();
        let skills = vec![make_local_skill("1.2.0", "local-sha")];
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated_for_skills(&skills, &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].upgrade_available);
    }

    #[test]
    fn no_upgrade_when_registry_is_older() {
        let cfg = make_config();
        let skills = vec![make_local_skill("2.0.0", "local-sha")];
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated_for_skills(&skills, &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].upgrade_available);
    }

    #[test]
    fn skips_skills_not_in_registry() {
        let cfg = make_config();
        let f = FakeRegistry(Registry {
            hub: "h".into(),
            generated_at: "x".into(),
            schema_version: 1,
            skills: BTreeMap::new(),
        });
        let skills = vec![make_local_skill("1.0.0", "local-sha")];
        let rows = outdated_for_skills(&skills, &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn no_rows_when_no_remotes_configured() {
        let cfg = Config::default();
        let f = FakeRegistry(make_registry("2.0.0"));
        let skills = vec![make_local_skill("1.0.0", "local-sha")];
        let rows = outdated_for_skills(&skills, &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn locked_flag_reflects_lockfile_membership() {
        let cfg = make_config();
        let f = FakeRegistry(make_registry("1.2.0"));
        let skills = vec![make_local_skill("1.0.0", "local-sha")];
        // `make_local_skill` names the skill "csv-parse" (see helper). Not locked:
        let unlocked = outdated_for_skills(&skills, &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(!unlocked[0].locked);
        // Locked when its name is in the set:
        let locked_names: BTreeSet<String> =
            [unlocked[0].name.clone()].into_iter().collect();
        let locked = outdated_for_skills(&skills, &cfg, &f, &locked_names).unwrap();
        assert!(locked[0].locked);
    }
}
