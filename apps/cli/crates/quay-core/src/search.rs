use crate::config::Config;
use crate::error::Result;
use crate::fetcher::RegistryFetcher;
use crate::registry::RegistryEntry;
use serde::Serialize;

/// One row of `quay search` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub remote: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
}

/// Filters applied to the search.
#[derive(Debug, Clone, Default)]
pub struct SearchFilters<'a> {
    /// Free-text query — matches against name and description (case-insensitive substring).
    /// Empty string matches everything.
    pub query: &'a str,
    /// If set, only consider this remote name (must exist in config).
    pub remote: Option<&'a str>,
    /// If set, the entry must include this tag (case-insensitive).
    pub tag: Option<&'a str>,
}

/// Run the search across configured remotes. Results are sorted by `(remote, name)`.
pub fn search<R: RegistryFetcher>(
    config: &Config,
    fetcher: &R,
    filters: &SearchFilters<'_>,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    let q = filters.query.to_lowercase();
    let tag = filters.tag.map(|t| t.to_lowercase());

    let remote_names: Vec<&String> = match filters.remote {
        Some(name) => {
            if !config.remotes.contains_key(name) {
                return Err(crate::error::QuayError::RemoteUnknown(name.into()));
            }
            config
                .remotes
                .keys()
                .filter(|k| k.as_str() == name)
                .collect()
        }
        None => config.remotes.keys().collect(),
    };

    for remote_name in remote_names {
        let url = &config.remotes[remote_name].url;
        let registry = fetcher.fetch(url)?;
        for (name, entry) in &registry.skills {
            if !matches_filters(name, entry, &q, tag.as_deref()) {
                continue;
            }
            hits.push(SearchHit {
                remote: remote_name.clone(),
                name: name.clone(),
                version: entry.version.clone(),
                description: entry.description.clone(),
                category: entry.category.clone(),
                tags: entry.tags.clone(),
            });
        }
    }

    // Stable, deterministic ordering: by (remote, name).
    hits.sort_by(|a, b| {
        (a.remote.as_str(), a.name.as_str()).cmp(&(b.remote.as_str(), b.name.as_str()))
    });
    Ok(hits)
}

fn matches_filters(
    name: &str,
    entry: &RegistryEntry,
    query_lower: &str,
    tag_lower: Option<&str>,
) -> bool {
    if !query_lower.is_empty() {
        let in_name = name.to_lowercase().contains(query_lower);
        let in_desc = entry.description.to_lowercase().contains(query_lower);
        if !in_name && !in_desc {
            return false;
        }
    }
    if let Some(want_tag) = tag_lower {
        if !entry.tags.iter().any(|t| t.to_lowercase() == want_tag) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfig;
    use crate::registry::{Registry, RegistryEntry};

    struct FakeRegistry(Registry);
    impl RegistryFetcher for FakeRegistry {
        fn fetch(&self, _url: &str) -> Result<Registry> {
            Ok(self.0.clone())
        }
    }

    fn make_entry(
        version: &str,
        description: &str,
        tags: &[&str],
        category: Option<&str>,
    ) -> RegistryEntry {
        RegistryEntry {
            version: version.into(),
            description: description.into(),
            category: category.map(Into::into),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            path: "skills/x".into(),
            sha: "0".into(),
            files: vec!["SKILL.md".into()],
        }
    }

    fn make_config_with_one_remote() -> Config {
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

    fn make_registry(skills: &[(&str, RegistryEntry)]) -> Registry {
        Registry {
            hub: "h".into(),
            generated_at: "2026-05-08T00:00:00Z".into(),
            schema_version: 1,
            skills: skills
                .iter()
                .map(|(n, e)| ((*n).into(), e.clone()))
                .collect(),
        }
    }

    #[test]
    fn empty_query_returns_all() {
        let cfg = make_config_with_one_remote();
        let reg = make_registry(&[
            (
                "csv-parse",
                make_entry("1.0.0", "Parse CSV.", &["data"], Some("backend")),
            ),
            (
                "json-clean",
                make_entry("0.5.0", "Clean JSON.", &["data"], Some("backend")),
            ),
        ]);
        let f = FakeRegistry(reg);
        let hits = search(
            &cfg,
            &f,
            &SearchFilters {
                query: "",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "csv-parse");
        assert_eq!(hits[1].name, "json-clean");
    }

    #[test]
    fn query_matches_name_or_description_case_insensitive() {
        let cfg = make_config_with_one_remote();
        let reg = make_registry(&[
            (
                "csv-parse",
                make_entry("1.0.0", "Parse CSV files.", &[], None),
            ),
            (
                "json-clean",
                make_entry("0.5.0", "Clean JSON content.", &[], None),
            ),
        ]);
        let f = FakeRegistry(reg);
        let hits = search(
            &cfg,
            &f,
            &SearchFilters {
                query: "JSON",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "json-clean");

        let hits = search(
            &cfg,
            &f,
            &SearchFilters {
                query: "csv",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "csv-parse");
    }

    #[test]
    fn tag_filter_requires_exact_tag() {
        let cfg = make_config_with_one_remote();
        let reg = make_registry(&[
            ("a", make_entry("1.0.0", "x", &["data", "parsing"], None)),
            ("b", make_entry("1.0.0", "x", &["ui"], None)),
        ]);
        let f = FakeRegistry(reg);
        let hits = search(
            &cfg,
            &f,
            &SearchFilters {
                query: "",
                tag: Some("DATA"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "a");
    }

    #[test]
    fn pinned_remote_filter_excludes_others() {
        let mut cfg = make_config_with_one_remote();
        cfg.remotes.insert(
            "other".into(),
            RemoteConfig {
                url: "https://github.com/x/y.git".into(),
                default: false,
                provider: None,
            },
        );
        let reg = make_registry(&[("a", make_entry("1.0.0", "x", &[], None))]);
        let f = FakeRegistry(reg);
        let hits = search(
            &cfg,
            &f,
            &SearchFilters {
                query: "",
                remote: Some("h"),
                ..Default::default()
            },
        )
        .unwrap();
        // The fake fetcher returns the same registry for any URL, so we know it would have
        // returned 2 results if both remotes were queried. With remote filter, only "h" runs.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].remote, "h");
    }

    #[test]
    fn unknown_pinned_remote_errors() {
        let cfg = make_config_with_one_remote();
        let reg = make_registry(&[]);
        let f = FakeRegistry(reg);
        let err = search(
            &cfg,
            &f,
            &SearchFilters {
                remote: Some("nope"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, crate::error::QuayError::RemoteUnknown(_)));
    }

    #[test]
    fn results_sorted_by_remote_then_name() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "z-hub".into(),
            RemoteConfig {
                url: "https://github.com/z/z.git".into(),
                default: false,
                provider: None,
            },
        );
        cfg.remotes.insert(
            "a-hub".into(),
            RemoteConfig {
                url: "https://github.com/a/a.git".into(),
                default: false,
                provider: None,
            },
        );
        let reg = make_registry(&[
            ("zebra", make_entry("1.0.0", "x", &[], None)),
            ("apple", make_entry("1.0.0", "x", &[], None)),
        ]);
        let f = FakeRegistry(reg);
        let hits = search(&cfg, &f, &SearchFilters::default()).unwrap();
        // 4 hits: a-hub/apple, a-hub/zebra, z-hub/apple, z-hub/zebra
        let order: Vec<(String, String)> = hits
            .iter()
            .map(|h| (h.remote.clone(), h.name.clone()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("a-hub".into(), "apple".into()),
                ("a-hub".into(), "zebra".into()),
                ("z-hub".into(), "apple".into()),
                ("z-hub".into(), "zebra".into()),
            ]
        );
    }
}
