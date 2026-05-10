//! Search query parser for the TUI search bar.
//!
//! Tokens are space-separated and combined as logical AND.
//!
//! Supported syntax:
//! - bare word `foo` — name (folder) substring match (case-insensitive)
//! - `#tag` — frontmatter `tags` array contains `tag`
//! - `status:local` / `status:installed` / `status:installed-modified` / `status:pushed-local`
//! - `mirror:agents` / `mirror:claude` / `mirror:codex` / `mirror:cursor`
//! - `remote:<name>` — restrict to a specific remote (Remote pane only)

use quay_core::scanner::{LocalSkill, ScanStatus};
use quay_core::MirrorRoot;

/// Parsed search query ready for filtering.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Bare-word name substrings (case-insensitive AND).
    pub name_contains: Vec<String>,
    /// Tag values that must be present in the skill's frontmatter tags.
    pub tags: Vec<String>,
    /// Status filters — if non-empty, skill status must match one of them.
    pub statuses: Vec<StatusFilter>,
    /// Mirror filters — if non-empty, skill must appear in at least one of these mirrors.
    pub mirrors: Vec<MirrorRoot>,
    /// Remote name filter (Remote pane only).
    pub remotes: Vec<String>,
}

/// Status token values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusFilter {
    Local,
    Installed,
    InstalledModified,
    PushedLocal,
}

impl SearchQuery {
    /// Parse a raw query string into a [`SearchQuery`].
    ///
    /// Unknown tokens are silently dropped so partial typing does not break filtering.
    pub fn parse(raw: &str) -> Self {
        let mut q = SearchQuery::default();
        for token in raw.split_whitespace() {
            if let Some(tag) = token.strip_prefix('#') {
                if !tag.is_empty() {
                    q.tags.push(tag.to_ascii_lowercase());
                }
            } else if let Some(val) = token.strip_prefix("status:") {
                match val {
                    "local" => q.statuses.push(StatusFilter::Local),
                    "installed" => q.statuses.push(StatusFilter::Installed),
                    "installed-modified" => q.statuses.push(StatusFilter::InstalledModified),
                    "pushed-local" => q.statuses.push(StatusFilter::PushedLocal),
                    _ => {}
                }
            } else if let Some(val) = token.strip_prefix("mirror:") {
                match val {
                    "agents" => q.mirrors.push(MirrorRoot::Agents),
                    "claude" => q.mirrors.push(MirrorRoot::Claude),
                    "codex" => q.mirrors.push(MirrorRoot::Codex),
                    "cursor" => q.mirrors.push(MirrorRoot::Cursor),
                    _ => {}
                }
            } else if let Some(val) = token.strip_prefix("remote:") {
                if !val.is_empty() {
                    q.remotes.push(val.to_ascii_lowercase());
                }
            } else if !token.is_empty() {
                q.name_contains.push(token.to_ascii_lowercase());
            }
        }
        q
    }

    /// Returns `true` if the query has no constraints (matches everything).
    pub fn is_empty(&self) -> bool {
        self.name_contains.is_empty()
            && self.tags.is_empty()
            && self.statuses.is_empty()
            && self.mirrors.is_empty()
            && self.remotes.is_empty()
    }

    /// Returns `true` if `skill` matches all constraints in this query.
    pub fn matches_local(&self, skill: &LocalSkill) -> bool {
        // Name substring.
        let name_lower = skill.meta.name.to_ascii_lowercase();
        for sub in &self.name_contains {
            if !name_lower.contains(sub.as_str()) {
                return false;
            }
        }

        // Tags.
        for required_tag in &self.tags {
            let skill_tags_lower: Vec<String> = skill
                .meta
                .tags
                .iter()
                .map(|t| t.to_ascii_lowercase())
                .collect();
            if !skill_tags_lower.iter().any(|t| t == required_tag) {
                return false;
            }
        }

        // Status.
        if !self.statuses.is_empty() {
            let status_matches = self.statuses.iter().any(|sf| match sf {
                StatusFilter::Local => matches!(skill.status, ScanStatus::Local),
                StatusFilter::Installed => matches!(skill.status, ScanStatus::Installed { .. }),
                StatusFilter::InstalledModified => {
                    matches!(skill.status, ScanStatus::InstalledModified { .. })
                }
                StatusFilter::PushedLocal => matches!(skill.status, ScanStatus::PushedLocal { .. }),
            });
            if !status_matches {
                return false;
            }
        }

        // Mirror.
        if !self.mirrors.is_empty() {
            let has_mirror = self
                .mirrors
                .iter()
                .any(|m| skill.locations.iter().any(|l| l.root == *m));
            if !has_mirror {
                return false;
            }
        }

        // `remote:` tokens are Remote-pane only; no-op for LocalSkill.

        true
    }
}

/// Filter a slice of `LocalSkill` using a query.
///
/// Returns indices (into the original slice) of skills that match.
pub fn filter_local(skills: &[LocalSkill], query: &SearchQuery) -> Vec<usize> {
    if query.is_empty() {
        return (0..skills.len()).collect();
    }
    skills
        .iter()
        .enumerate()
        .filter(|(_, s)| query.matches_local(s))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_empty_string_gives_empty_query() {
        let q = SearchQuery::parse("");
        assert!(q.is_empty());
    }

    #[test]
    fn parse_bare_word_goes_into_name_contains() {
        let q = SearchQuery::parse("foo");
        assert_eq!(q.name_contains, vec!["foo".to_string()]);
        assert!(q.tags.is_empty());
    }

    #[test]
    fn parse_multiple_bare_words() {
        let q = SearchQuery::parse("foo bar");
        assert_eq!(q.name_contains, vec!["foo", "bar"]);
    }

    #[test]
    fn parse_tag_prefix() {
        let q = SearchQuery::parse("#api");
        assert_eq!(q.tags, vec!["api".to_string()]);
        assert!(q.name_contains.is_empty());
    }

    #[test]
    fn parse_status_installed() {
        let q = SearchQuery::parse("status:installed");
        assert_eq!(q.statuses, vec![StatusFilter::Installed]);
    }

    #[test]
    fn parse_status_local() {
        let q = SearchQuery::parse("status:local");
        assert_eq!(q.statuses, vec![StatusFilter::Local]);
    }

    #[test]
    fn parse_status_installed_modified() {
        let q = SearchQuery::parse("status:installed-modified");
        assert_eq!(q.statuses, vec![StatusFilter::InstalledModified]);
    }

    #[test]
    fn parse_status_pushed_local() {
        let q = SearchQuery::parse("status:pushed-local");
        assert_eq!(q.statuses, vec![StatusFilter::PushedLocal]);
    }

    #[test]
    fn parse_mirror_claude() {
        let q = SearchQuery::parse("mirror:claude");
        assert_eq!(q.mirrors, vec![MirrorRoot::Claude]);
    }

    #[test]
    fn parse_mirror_agents() {
        let q = SearchQuery::parse("mirror:agents");
        assert_eq!(q.mirrors, vec![MirrorRoot::Agents]);
    }

    #[test]
    fn parse_remote_token() {
        let q = SearchQuery::parse("remote:my-hub");
        assert_eq!(q.remotes, vec!["my-hub".to_string()]);
    }

    #[test]
    fn parse_combined_query() {
        let q = SearchQuery::parse("foo #api status:installed");
        assert_eq!(q.name_contains, vec!["foo"]);
        assert_eq!(q.tags, vec!["api"]);
        assert_eq!(q.statuses, vec![StatusFilter::Installed]);
    }

    #[test]
    fn parse_unknown_status_is_silently_dropped() {
        let q = SearchQuery::parse("status:unknown_value");
        assert!(q.statuses.is_empty());
    }

    #[test]
    fn parse_unknown_mirror_is_silently_dropped() {
        let q = SearchQuery::parse("mirror:kimi");
        assert!(q.mirrors.is_empty());
    }

    // -----------------------------------------------------------------------
    // Filter tests
    // -----------------------------------------------------------------------

    fn make_local_skill(name: &str, tags: &[&str], status: ScanStatus) -> LocalSkill {
        use quay_core::scanner::{LocalLocation, SkillFormat, SkillMeta};
        LocalSkill {
            meta: SkillMeta {
                name: name.to_string(),
                description: format!("desc of {name}"),
                version: "0.1.0".to_string(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                format: SkillFormat::Frontmatter,
            },
            locations: vec![LocalLocation {
                root: MirrorRoot::Agents,
                path: std::path::PathBuf::from(format!(".agents/skills/{name}/SKILL.md")),
                sha256: "abc".to_string(),
            }],
            status,
        }
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let skills = vec![
            make_local_skill("foo", &[], ScanStatus::Local),
            make_local_skill("bar", &[], ScanStatus::Local),
        ];
        let q = SearchQuery::parse("");
        let result = filter_local(&skills, &q);
        assert_eq!(result, vec![0, 1]);
    }

    #[test]
    fn filter_name_contains_case_insensitive() {
        let skills = vec![
            make_local_skill("csv-parse", &[], ScanStatus::Local),
            make_local_skill("json-decode", &[], ScanStatus::Local),
        ];
        let q = SearchQuery::parse("CSV");
        let result = filter_local(&skills, &q);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn filter_tag_match() {
        let skills = vec![
            make_local_skill("a", &["api", "json"], ScanStatus::Local),
            make_local_skill("b", &["csv"], ScanStatus::Local),
        ];
        let q = SearchQuery::parse("#api");
        let result = filter_local(&skills, &q);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn filter_status_local() {
        let skills = vec![
            make_local_skill("a", &[], ScanStatus::Local),
            make_local_skill(
                "b",
                &[],
                ScanStatus::Installed {
                    remote: "r".into(),
                    version: "1.0.0".into(),
                },
            ),
        ];
        let q = SearchQuery::parse("status:local");
        let result = filter_local(&skills, &q);
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn filter_mirror_agents() {
        use quay_core::scanner::{LocalLocation, SkillFormat, SkillMeta};
        let claude_skill = LocalSkill {
            meta: SkillMeta {
                name: "claude-only".to_string(),
                description: "d".to_string(),
                version: "0.1.0".to_string(),
                tags: vec![],
                format: SkillFormat::Frontmatter,
            },
            locations: vec![LocalLocation {
                root: MirrorRoot::Claude,
                path: ".claude/skills/claude-only/SKILL.md".into(),
                sha256: "abc".to_string(),
            }],
            status: ScanStatus::Local,
        };
        let agents_skill = make_local_skill("agents-only", &[], ScanStatus::Local);
        let skills = vec![claude_skill, agents_skill];
        let q = SearchQuery::parse("mirror:agents");
        let result = filter_local(&skills, &q);
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn filter_combined_and_semantics() {
        let skills = vec![
            make_local_skill("csv-parse", &["api"], ScanStatus::Local),
            make_local_skill("csv-export", &["csv"], ScanStatus::Local),
            make_local_skill("json-api", &["api"], ScanStatus::Local),
        ];
        // Must match "csv" AND "#api".
        let q = SearchQuery::parse("csv #api");
        let result = filter_local(&skills, &q);
        assert_eq!(result, vec![0]);
    }
}
