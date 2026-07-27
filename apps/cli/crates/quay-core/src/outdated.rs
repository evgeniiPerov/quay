//! On-the-fly outdated detection: compare local-file SHA-256 against the
//! remote hub's registry entry.
//!
//! A `skills-lock.json` now records what is installed and from where (see the
//! `lock` module).
//!
//! Two comparisons run per row, and they answer different questions.
//!
//! **Content** (`content_drift`) is compared for every format: the hub entry's
//! `content_hash` against `LocalSkill::content_hash`. Bumping `version` on push
//! is a convention quay does not enforce, so this is the only signal that
//! catches a hub edit shipped at an unchanged version. It is direction-neutral
//! — two hashes prove the bytes differ, not which side moved them. The local
//! side is also hashed with line endings normalized
//! (`LocalSkill::content_hash_lf`) and either digest matching counts as
//! unchanged, because git's `core.autocrlf` on Windows would otherwise make
//! every installed skill look drifted.
//!
//! **Version** (`upgrade_available`) is semver for frontmatter skills.
//! Hand-written skills (`SlashCommand` and `Freestyle`) have no semver, so for
//! them content drift *is* the upgrade signal.
//!
//! A missing `content_hash` on a hub entry means the hub registry predates
//! content-hash indexing — no comparison is possible, so nothing is flagged (no
//! false positives) and hand-written rows display `unversioned`. The same
//! never-flag fallback applies when the local content hash cannot be computed
//! (e.g. an unreadable file); that case additionally logs a warning to stderr.
//!
//! The `sha` field remains an informational, git-object-SHA column. The
//! lockfile contributes a `locked` flag per row and offline content-hash drift
//! detection via `quay lock --check` — note that check uses `lock_hash`'s
//! whole-folder digest, a different hash space from the pushable content hash
//! compared here.

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
    /// Display value for the "available" column: the remote semver for
    /// frontmatter skills, or a short content-hash / `"unversioned"` for
    /// hand-written skills (see `by_content_hash`).
    pub available: String,
    /// SHA-256 of the local canonical `SKILL.md`.
    pub local_sha: String,
    /// SHA from `registry.json` (`entry.sha`).
    pub remote_sha: String,
    /// True when an upgrade is available: for frontmatter skills, `available`
    /// is a higher semver than the local version; for hand-written skills
    /// (`by_content_hash == true`), the hub's content hash differs from the
    /// local content hash.
    pub upgrade_available: bool,
    /// True when this skill is recorded in `skills-lock.json`.
    pub locked: bool,
    /// True when this row was compared by content hash (hand-written skill)
    /// rather than semver.
    #[serde(default)]
    pub by_content_hash: bool,
    /// True when the local bytes differ from the hub's recorded
    /// `content_hash`, regardless of what the versions say. Set for every
    /// format, so a frontmatter skill whose hub copy was edited without a
    /// version bump no longer reports as up to date.
    ///
    /// **Direction-neutral by design.** "Differs" is all two hashes can prove:
    /// a hub-side edit and a local-side edit are the same observation from
    /// here. The lockfile cannot break the tie — its `computed_hash` is a
    /// `lock_hash::folder_hash` (dotfiles included, for vercel interop) while
    /// this compares `skill_files::pushable_content_hash`, so the two are not
    /// in the same hash space. Deciding which side moved needs harbor history
    /// (see the `reconcile` module).
    ///
    /// False when the comparison could not be made at all: a hub registry
    /// predating content-hash indexing, or an unreadable local skill.
    #[serde(default)]
    pub content_drift: bool,
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
    let locked_names: BTreeSet<String> = match crate::lock::read(project_root)? {
        Some(lock) => lock.skills.keys().cloned().collect(),
        None => BTreeSet::new(),
    };
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
        // Hashing walks the skill folder, so do it at most once per skill
        // rather than once per remote. Outer `None` means "not computed yet";
        // inner `None` means the walk failed and was already warned about.
        let mut local_hash: Option<Option<(String, String)>> = None;

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

            // Content comparison, made for every format. `None` means the
            // comparison was impossible: either the hub registry predates
            // content-hash indexing, or the local skill could not be hashed. A
            // local read failure is indistinguishable from "changed" if it
            // defaults to empty, so it stays unknown rather than being flagged
            // — but it warns, so a corrupt install is not silent.
            let remote_hash = &entry.content_hash;
            let drift: Option<bool> = if remote_hash.is_empty() {
                None
            } else {
                let computed = local_hash.get_or_insert_with(|| {
                    // Raw and LF-normalized. Matching either means "unchanged":
                    // a Windows checkout differs from the hub's LF bytes only in
                    // line endings, and flagging that would light up every row
                    // on the platform.
                    match (skill.content_hash(), skill.content_hash_lf()) {
                        (Ok(raw), Ok(lf)) => Some((raw, lf)),
                        (Err(e), _) | (_, Err(e)) => {
                            eprintln!(
                                "warning: could not hash {}: {e}; skipping content-hash comparison",
                                skill.meta.name
                            );
                            None
                        }
                    }
                });
                computed
                    .as_ref()
                    .map(|(raw, lf)| remote_hash != raw && remote_hash != lf)
            };

            let (available, upgrade_available, by_content_hash) = if matches!(
                skill.meta.format,
                crate::scanner::SkillFormat::SlashCommand | crate::scanner::SkillFormat::Freestyle
            ) {
                // Hand-written skill: no semver, so content drift *is* the
                // upgrade signal. An impossible comparison displays as
                // `unversioned` and never flags (no false positives).
                match drift {
                    Some(changed) => (short_hash(remote_hash), changed, true),
                    None => (String::from("unversioned"), false, true),
                }
            } else {
                // Frontmatter skill: semver decides whether an *upgrade* is
                // available. Drift is reported separately — bumping the version
                // is a convention, not something quay enforces on push, so an
                // equal-version hub edit must not read as up to date.
                let up = match (
                    Version::parse(&entry.version),
                    Version::parse(&local_version),
                ) {
                    (Ok(av), Ok(loc)) => av.cmp(&loc) == Ordering::Greater,
                    _ => false,
                };
                (entry.version.clone(), up, false)
            };

            rows.push(OutdatedEntry {
                name: skill.meta.name.clone(),
                remote: remote_name.clone(),
                available,
                local_sha: local_sha.clone(),
                remote_sha: entry.sha.clone(),
                upgrade_available,
                locked: locked_names.contains(&skill.meta.name),
                by_content_hash,
                content_drift: drift.unwrap_or(false),
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

/// First 12 hex chars of a content hash, for display. Only called with a
/// non-empty hash — the caller special-cases the empty/unknown case.
fn short_hash(h: &str) -> String {
    h.chars().take(12).collect()
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
                    content_hash: String::new(),
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
        let locked_names: BTreeSet<String> = [unlocked[0].name.clone()].into_iter().collect();
        let locked = outdated_for_skills(&skills, &cfg, &f, &locked_names).unwrap();
        assert!(locked[0].locked);
    }

    fn freestyle_skill_on_disk(dir: &std::path::Path, body: &str) -> LocalSkill {
        non_frontmatter_skill_on_disk(dir, body, SkillFormat::Freestyle)
    }

    fn non_frontmatter_skill_on_disk(
        dir: &std::path::Path,
        body: &str,
        format: SkillFormat,
    ) -> LocalSkill {
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
        LocalSkill {
            meta: SkillMeta {
                name: "csv-parse".into(),
                description: "Parse CSV.".into(),
                version: "0.0.0".into(),
                tags: vec![],
                format,
            },
            locations: vec![LocalLocation {
                root: crate::config::MirrorRoot::Agents,
                path: dir.join("SKILL.md"),
                sha256: "irrelevant".into(),
            }],
            status: ScanStatus::Local,
        }
    }

    fn registry_with_content_hash(name: &str, content_hash: &str) -> Registry {
        Registry {
            hub: "h".into(),
            generated_at: "x".into(),
            schema_version: 1,
            skills: BTreeMap::from([(
                name.into(),
                RegistryEntry {
                    version: "0.0.0".into(),
                    description: "Parse CSV.".into(),
                    category: None,
                    tags: vec![],
                    path: format!("skills/{name}"),
                    sha: "sha".into(),
                    files: vec!["SKILL.md".into()],
                    source_format: SkillFormat::Freestyle,
                    content_hash: content_hash.into(),
                },
            )]),
        }
    }

    fn frontmatter_skill_on_disk(dir: &std::path::Path, body: &str, version: &str) -> LocalSkill {
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
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
                path: dir.join("SKILL.md"),
                sha256: "irrelevant".into(),
            }],
            status: ScanStatus::Local,
        }
    }

    fn frontmatter_registry(version: &str, content_hash: &str) -> Registry {
        let mut r = make_registry(version);
        r.skills.get_mut("csv-parse").unwrap().content_hash = content_hash.into();
        r
    }

    #[test]
    fn frontmatter_drifts_when_hub_edited_without_a_version_bump() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill =
            frontmatter_skill_on_disk(tmp.path(), "---\nname: csv-parse\n---\nbody\n", "1.0.0");
        let cfg = make_config();
        // Same version on both sides; the hub's bytes differ.
        let f = FakeRegistry(frontmatter_registry("1.0.0", &"a".repeat(64)));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            !rows[0].upgrade_available,
            "versions are equal, so this is not a semver upgrade"
        );
        assert!(
            rows[0].content_drift,
            "hub content hash differs from local, so the row must report drift"
        );
    }

    #[test]
    fn frontmatter_does_not_drift_when_content_matches() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill =
            frontmatter_skill_on_disk(tmp.path(), "---\nname: csv-parse\n---\nbody\n", "1.0.0");
        let hash = skill.content_hash().unwrap();
        let cfg = make_config();
        let f = FakeRegistry(frontmatter_registry("1.0.0", &hash));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(!rows[0].content_drift);
        assert!(!rows[0].upgrade_available);
    }

    #[test]
    fn crlf_only_difference_is_not_drift() {
        // git's default core.autocrlf on Windows hands back CRLF at checkout,
        // so the local bytes differ from the LF bytes the hub hashed. Every
        // skill would report drift on Windows if that counted.
        let lf = "---\nname: csv-parse\n---\nbody\nmore\n";
        let hub = tempfile::TempDir::new().unwrap();
        let hub_skill = frontmatter_skill_on_disk(hub.path(), lf, "1.0.0");
        let hub_hash = hub_skill.content_hash().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let skill = frontmatter_skill_on_disk(tmp.path(), &lf.replace('\n', "\r\n"), "1.0.0");
        let cfg = make_config();
        let f = FakeRegistry(frontmatter_registry("1.0.0", &hub_hash));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(
            !rows[0].content_drift,
            "line endings alone must not read as a content change"
        );
    }

    #[test]
    fn frontmatter_never_drifts_when_hub_registry_has_no_content_hash() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill =
            frontmatter_skill_on_disk(tmp.path(), "---\nname: csv-parse\n---\nbody\n", "1.0.0");
        let cfg = make_config();
        // `make_registry` leaves `content_hash` empty — a registry.json written
        // before content-hash indexing. Nothing to compare against.
        let f = FakeRegistry(make_registry("1.0.0"));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(!rows[0].content_drift, "no hub hash means no claim");
    }

    #[test]
    fn frontmatter_reports_drift_and_upgrade_together_when_hub_is_newer() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill =
            frontmatter_skill_on_disk(tmp.path(), "---\nname: csv-parse\n---\nbody\n", "1.0.0");
        let cfg = make_config();
        let f = FakeRegistry(frontmatter_registry("2.0.0", &"a".repeat(64)));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(rows[0].upgrade_available, "2.0.0 > 1.0.0");
        assert!(
            rows[0].content_drift,
            "drift is reported alongside an upgrade, not instead of it"
        );
    }

    #[test]
    fn frontmatter_drifts_when_local_version_is_ahead_but_content_differs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill =
            frontmatter_skill_on_disk(tmp.path(), "---\nname: csv-parse\n---\nbody\n", "3.0.0");
        let cfg = make_config();
        let f = FakeRegistry(frontmatter_registry("1.0.0", &"a".repeat(64)));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert!(!rows[0].upgrade_available, "local semver is ahead");
        assert!(rows[0].content_drift, "the bytes still differ");
    }

    #[test]
    fn non_frontmatter_up_to_date_when_hashes_match() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = freestyle_skill_on_disk(tmp.path(), "# /csv-parse\nbody\n");
        let hash = skill.content_hash().unwrap();
        let cfg = make_config();
        let f = FakeRegistry(registry_with_content_hash("csv-parse", &hash));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].by_content_hash);
        assert!(!rows[0].upgrade_available);
    }

    #[test]
    fn non_frontmatter_upgrade_when_hashes_differ() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = freestyle_skill_on_disk(tmp.path(), "# /csv-parse\nbody\n");
        let cfg = make_config();
        // Registry carries a different (stale) hash than what's on disk now.
        let f = FakeRegistry(registry_with_content_hash("csv-parse", &"a".repeat(64)));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].by_content_hash);
        assert!(rows[0].upgrade_available);
        assert!(
            rows[0].content_drift,
            "content_drift is set for every format, so one field answers \
             'do my bytes differ from the hub' regardless of skill kind"
        );
    }

    #[test]
    fn non_frontmatter_slashcommand_upgrade_when_hashes_differ() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = non_frontmatter_skill_on_disk(
            tmp.path(),
            "# /csv-parse\nbody\n",
            SkillFormat::SlashCommand,
        );
        let cfg = make_config();
        // Registry carries a different (stale) hash than what's on disk now.
        let f = FakeRegistry(registry_with_content_hash("csv-parse", &"a".repeat(64)));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].by_content_hash);
        assert!(rows[0].upgrade_available);
    }

    #[test]
    fn non_frontmatter_no_flag_when_registry_lacks_content_hash() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = freestyle_skill_on_disk(tmp.path(), "# /csv-parse\nbody\n");
        let cfg = make_config();
        let f = FakeRegistry(registry_with_content_hash("csv-parse", "")); // legacy hub
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].upgrade_available); // unknown → never a false positive
    }

    #[test]
    fn non_frontmatter_no_flag_when_local_hash_unreadable() {
        // Registry has a real (non-empty) content_hash, but the local skill
        // folder cannot be read (path points at a nonexistent dir), so
        // content_hash() errors. Must never flag (fail-safe), not report a
        // false "changed".
        let skill = LocalSkill {
            meta: SkillMeta {
                name: "csv-parse".into(),
                description: "Parse CSV.".into(),
                version: "0.0.0".into(),
                tags: vec![],
                format: SkillFormat::Freestyle,
            },
            locations: vec![LocalLocation {
                root: crate::config::MirrorRoot::Agents,
                path: std::path::PathBuf::from("/nonexistent-quay-test-dir/csv-parse/SKILL.md"),
                sha256: "irrelevant".into(),
            }],
            status: ScanStatus::Local,
        };
        let cfg = make_config();
        let f = FakeRegistry(registry_with_content_hash("csv-parse", &"a".repeat(64)));
        let rows = outdated_for_skills(&[skill], &cfg, &f, &BTreeSet::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].by_content_hash);
        assert!(!rows[0].upgrade_available); // read error → never flag
    }
}
