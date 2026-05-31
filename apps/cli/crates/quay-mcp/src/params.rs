//! Tool wire types. Param structs derive `Deserialize + JsonSchema`;
//! output structs derive `Serialize + JsonSchema`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Free-text query. Matches skill name and description (case-insensitive).
    /// Empty string returns everything.
    #[serde(default)]
    pub query: String,
    /// Restrict to one configured remote by name. Optional.
    #[serde(default)]
    pub remote: Option<String>,
    /// Require this tag (case-insensitive). Optional.
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResultRow {
    pub name: String,
    pub version: String,
    pub remote: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
}

/// Object wrapper for the search result list. MCP requires a tool's structured
/// output schema to have an object root, so the rows are nested under `results`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResults {
    pub results: Vec<SearchResultRow>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillRef {
    /// Skill name.
    pub skill: String,
    /// Restrict to one configured remote. Optional.
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InstalledSkill {
    pub name: String,
    pub path: String,
}

/// Object wrapper — rmcp 1.7 requires object-rooted tool output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InstalledSkills {
    pub skills: Vec<InstalledSkill>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScannedLocation {
    pub name: String,
    /// Absolute path to the SKILL.md file.
    pub path: String,
    /// Which mirror root this location belongs to.
    pub root: String,
}

/// Object wrapper — rmcp 1.7 requires object-rooted tool output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScanReport {
    pub locations: Vec<ScannedLocation>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OutdatedRow {
    pub name: String,
    pub remote: String,
    /// SHA-256 of the locally installed SKILL.md.
    pub local_sha: String,
    /// SHA recorded in the hub's registry.json.
    pub remote_sha: String,
    /// Version available on the hub.
    pub available: String,
}

/// Object wrapper — rmcp 1.7 requires object-rooted tool output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OutdatedReport {
    pub outdated: Vec<OutdatedRow>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateParams {
    /// Skill name (canonical install). Validates its SKILL.md frontmatter.
    pub skill: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidateResult {
    pub ok: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddParams {
    /// Skill name to install.
    pub skill: String,
    /// Pin to one remote. Optional.
    #[serde(default)]
    pub remote: Option<String>,
    /// Overwrite if already installed.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillName {
    /// Skill name.
    pub skill: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteResult {
    pub ok: bool,
    /// Human-readable summary of what changed.
    pub message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PushParams {
    /// Skill name to publish.
    pub skill: String,
    /// Target remote. Optional (uses default remote).
    #[serde(default)]
    pub remote: Option<String>,
    /// Version bump: "patch", "minor", or "major". Optional (defaults to patch).
    #[serde(default)]
    pub bump: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PushOutcome {
    pub ok: bool,
    /// PR URL or compare URL, when available.
    pub url: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoteAddParams {
    /// Local name for the remote.
    pub name: String,
    /// Git URL of the hub.
    pub url: String,
    /// Make this the default remote.
    #[serde(default)]
    pub default: bool,
}
