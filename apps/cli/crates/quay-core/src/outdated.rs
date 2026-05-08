use crate::config::Config;
use crate::error::Result;
use crate::fetcher::RegistryFetcher;
use crate::lockfile::Lockfile;
use semver::Version;
use serde::Serialize;
use std::cmp::Ordering;

/// One row of `quay outdated` / `quay update` output describing a single skill's status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutdatedEntry {
    pub name: String,
    pub remote: String,
    /// Version recorded in the lockfile (currently installed).
    pub installed: String,
    /// Version available in the remote registry right now.
    pub available: String,
    /// True when `available > installed` per semver.
    pub upgrade_available: bool,
}

/// Compare every entry in the lockfile against the current registry from each
/// referenced remote. Returns one [`OutdatedEntry`] per installed skill (including
/// skills that are already up to date — caller filters with `.upgrade_available` if
/// they only want stale ones).
///
/// Skills whose recorded `remote` is no longer in the user's config are silently
/// skipped — they cannot be checked because we have no URL to fetch from.
pub fn outdated<R: RegistryFetcher>(
    config: &Config,
    fetcher: &R,
    lockfile: &Lockfile,
) -> Result<Vec<OutdatedEntry>> {
    let mut rows = Vec::new();
    // Cache so we only fetch each remote's registry once even if many skills share it.
    let mut registry_cache: std::collections::BTreeMap<String, crate::registry::Registry> =
        std::collections::BTreeMap::new();

    for (skill_name, locked) in &lockfile.skills {
        let Some(remote_cfg) = config.remotes.get(&locked.remote) else {
            continue;
        };
        let registry = match registry_cache.get(&locked.remote) {
            Some(r) => r.clone(),
            None => {
                let r = fetcher.fetch(&remote_cfg.url)?;
                registry_cache.insert(locked.remote.clone(), r.clone());
                r
            }
        };
        let Some(entry) = registry.entry(skill_name) else {
            continue;
        };
        let upgrade_available = match (
            Version::parse(&entry.version),
            Version::parse(&locked.version),
        ) {
            (Ok(av), Ok(in_)) => av.cmp(&in_) == Ordering::Greater,
            _ => false,
        };
        rows.push(OutdatedEntry {
            name: skill_name.clone(),
            remote: locked.remote.clone(),
            installed: locked.version.clone(),
            available: entry.version.clone(),
            upgrade_available,
        });
    }

    rows.sort_by(|a, b| {
        (a.remote.as_str(), a.name.as_str()).cmp(&(b.remote.as_str(), b.name.as_str()))
    });
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfig;
    use crate::lockfile::{LockedFile, LockedRemote, LockedSkill, Lockfile};
    use crate::registry::{Registry, RegistryEntry};
    use std::collections::BTreeMap;

    struct FakeRegistry(Registry);
    impl RegistryFetcher for FakeRegistry {
        fn fetch(&self, _url: &str) -> Result<Registry> {
            Ok(self.0.clone())
        }
    }

    fn make_lockfile(installed_version: &str) -> Lockfile {
        Lockfile {
            lockfile_version: 1,
            remotes: BTreeMap::from([(
                "h".into(),
                LockedRemote {
                    url: "https://github.com/foo/bar.git".into(),
                    registry_sha: "x".into(),
                },
            )]),
            skills: BTreeMap::from([(
                "csv-parse".into(),
                LockedSkill {
                    remote: "h".into(),
                    version: installed_version.into(),
                    sha: "x".into(),
                    path: "skills/csv-parse".into(),
                    files: vec![LockedFile {
                        path: "SKILL.md".into(),
                        sha256: "0".repeat(64),
                    }],
                    installed_at: "2026-05-08T00:00:00Z".into(),
                },
            )]),
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
            },
        );
        cfg
    }

    fn make_registry(version: &str) -> Registry {
        Registry {
            hub: "h".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: BTreeMap::from([(
                "csv-parse".into(),
                RegistryEntry {
                    version: version.into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: "skills/csv-parse".into(),
                    sha: "abc".into(),
                    files: vec!["SKILL.md".into()],
                },
            )]),
        }
    }

    #[test]
    fn upgrade_available_when_registry_is_newer() {
        let cfg = make_config();
        let lock = make_lockfile("1.0.0");
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated(&cfg, &f, &lock).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].upgrade_available);
        assert_eq!(rows[0].installed, "1.0.0");
        assert_eq!(rows[0].available, "1.2.0");
    }

    #[test]
    fn no_upgrade_when_registry_matches() {
        let cfg = make_config();
        let lock = make_lockfile("1.2.0");
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated(&cfg, &f, &lock).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].upgrade_available);
    }

    #[test]
    fn no_upgrade_when_registry_is_older() {
        let cfg = make_config();
        let lock = make_lockfile("2.0.0");
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated(&cfg, &f, &lock).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].upgrade_available);
    }

    #[test]
    fn skips_skills_whose_remote_was_removed() {
        let cfg = Config::default();
        let lock = make_lockfile("1.0.0");
        let f = FakeRegistry(make_registry("1.2.0"));
        let rows = outdated(&cfg, &f, &lock).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn skips_skills_no_longer_in_registry() {
        let cfg = make_config();
        let lock = make_lockfile("1.0.0");
        let f = FakeRegistry(Registry {
            hub: "h".into(),
            generated_at: "x".into(),
            schema_version: 1,
            skills: BTreeMap::new(),
        });
        let rows = outdated(&cfg, &f, &lock).unwrap();
        assert!(rows.is_empty());
    }
}
