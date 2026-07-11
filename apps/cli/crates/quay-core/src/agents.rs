//! Agent registry — where each coding agent reads its skills.
//!
//! `data/agents.toml` is GENERATED from vercel-labs/skills `src/agents.ts` by
//! `scripts/sync-agents` (Node, CI-only). Runtime here is pure Rust: ship the
//! compiled table, feed it into the existing [`crate::linker`]. Upstream adds
//! an agent → CI regenerates the toml → PR. No Node for end users.

use crate::config::{InstallConfig, MirrorConfig, MirrorStrategy};
use crate::error::{QuayError, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The universal canonical dir — the single real copy every mirror points at.
/// Agents whose project dir *is* this need no mirror (they read it directly).
const CANONICAL: &str = ".agents/skills";

const AGENTS_TOML: &str = include_str!("../data/agents.toml");

#[derive(Debug, Deserialize)]
pub struct Registry {
    pub agents: BTreeMap<String, Agent>,
}

#[derive(Debug, Deserialize)]
pub struct Agent {
    pub display_name: String,
    /// Project-scope skills dir, repo-relative, e.g. `.claude/skills`.
    pub project: String,
    /// Global-scope skills dir as a template, e.g.
    /// `${CLAUDE_CONFIG_DIR:-~/.claude}/skills`. `None` = project scope only.
    pub global: Option<String>,
    /// Paths that, if any exists, mean this agent is installed on the machine.
    #[serde(default)]
    pub detect: Vec<String>,
    /// Set when detection isn't a plain OR-of-path-exists (e.g. `eve` needs a
    /// package.json dep check). Such agents are never auto-detected — users
    /// target them explicitly with `--agent`.
    // ponytail: only `eve` today; if more appear, dispatch on this string.
    #[serde(default)]
    pub detect_special: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum Scope {
    Project,
    Global,
}

/// Load the compiled registry. The toml is generated and its shape is asserted
/// in CI by the codegen self-check, so a parse failure here is a build bug.
pub fn registry() -> Registry {
    toml::from_str(AGENTS_TOML).expect("generated agents.toml must parse")
}

/// Build an [`InstallConfig`] for the chosen agents at the chosen scope.
///
/// `home` is injected (testable). The result plugs straight into
/// [`crate::linker::apply_all`] — canonical + mirrors, nothing new downstream.
/// Agents whose dir *is* the canonical are skipped (they read it directly).
pub fn install_config(
    reg: &Registry,
    agents: &[String],
    scope: Scope,
    home: &str,
) -> Result<InstallConfig> {
    let canonical = match scope {
        Scope::Project => PathBuf::from(CANONICAL),
        Scope::Global => PathBuf::from(resolve_template(&format!("~/{CANONICAL}"), home)),
    };
    let mut mirrors = Vec::new();
    for name in agents {
        let agent = reg
            .agents
            .get(name)
            .ok_or_else(|| QuayError::MirrorCheckFailed(format!("unknown agent: {name}")))?;
        let path = match scope {
            Scope::Project => PathBuf::from(&agent.project),
            Scope::Global => {
                let tmpl = agent.global.as_ref().ok_or_else(|| {
                    QuayError::MirrorCheckFailed(format!("{name} has no global scope"))
                })?;
                PathBuf::from(resolve_template(tmpl, home))
            }
        };
        if path == canonical {
            continue; // universal agent: reads canonical directly, no mirror
        }
        mirrors.push(MirrorConfig {
            path,
            strategy: MirrorStrategy::Auto,
        });
    }
    Ok(InstallConfig { canonical, mirrors })
}

/// Auto-detect installed agents: any whose `detect` path exists on disk.
pub fn detect_installed(reg: &Registry, home: &str) -> Vec<String> {
    reg.agents
        .iter()
        .filter(|(_, a)| {
            a.detect
                .iter()
                .any(|d| Path::new(&resolve_template(d, home)).exists())
        })
        .map(|(k, _)| k.clone())
        .collect()
}

/// Resolve one path template: expand every `${VAR:-fallback}` against the env,
/// then a leading `~` against `home`.
pub fn resolve_template(tmpl: &str, home: &str) -> String {
    let expanded = expand_env(tmpl);
    match expanded.strip_prefix('~') {
        Some(rest) => format!("{home}{rest}"),
        None => expanded,
    }
}

/// Expand `${VAR}` and `${VAR:-fallback}` using process env. Unset with no
/// fallback → empty.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let expr = &after[..end];
        let (var, fallback) = match expr.split_once(":-") {
            Some((v, f)) => (v, Some(f)),
            None => (expr, None),
        };
        match std::env::var(var) {
            Ok(v) if !v.is_empty() => out.push_str(&v),
            _ => out.push_str(fallback.unwrap_or("")),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_parses_and_has_many_agents() {
        let r = registry();
        assert!(r.agents.len() > 60, "expected >60 agents, got {}", r.agents.len());
    }

    #[test]
    fn claude_project_and_global_resolve() {
        let r = registry();
        let cfg = install_config(&r, &["claude-code".into()], Scope::Project, "/home/u").unwrap();
        assert_eq!(cfg.canonical, PathBuf::from(".agents/skills"));
        assert_eq!(cfg.mirrors[0].path, PathBuf::from(".claude/skills"));

        unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };
        let cfg = install_config(&r, &["claude-code".into()], Scope::Global, "/home/u").unwrap();
        assert_eq!(cfg.mirrors[0].path, PathBuf::from("/home/u/.claude/skills"));
    }

    #[test]
    fn env_override_wins_over_fallback() {
        unsafe { std::env::set_var("CODEX_HOME", "/opt/codex") };
        assert_eq!(
            resolve_template("${CODEX_HOME:-~/.codex}/skills", "/home/u"),
            "/opt/codex/skills"
        );
        unsafe { std::env::remove_var("CODEX_HOME") };
        assert_eq!(
            resolve_template("${CODEX_HOME:-~/.codex}/skills", "/home/u"),
            "/home/u/.codex/skills"
        );
    }

    #[test]
    fn universal_agent_emits_no_mirror() {
        let r = registry();
        // codex.project == .agents/skills == canonical → skipped
        let cfg = install_config(&r, &["codex".into()], Scope::Project, "/home/u").unwrap();
        assert!(cfg.mirrors.is_empty());
    }

    #[test]
    fn unknown_agent_errors() {
        let r = registry();
        assert!(install_config(&r, &["nope".into()], Scope::Project, "/home/u").is_err());
    }
}
