//! MCP tool surface. Every handler calls `quay-core` directly and returns
//! structured data. NEVER call `quay-cli` command functions here — they
//! print to stdout, which is the MCP protocol channel.

use crate::params::*;
use crate::ServeOptions;
use crate::ServerCtx;
use quay_core::linker;
use quay_core::push_log::PushLog;
use quay_core::scanner::scan_local;
use quay_core::{outdated_for_local, parse_skill, search, SearchFilters, SkillManager};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::ErrorData;
use rmcp::transport::io::stdio;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};

#[derive(Clone)]
pub struct QuayServer {
    ctx: ServerCtx,
    // Read by the `#[tool_handler]`-generated `ServerHandler` impl, which the
    // dead-code lint can't see through.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl QuayServer {
    pub fn new(opts: ServeOptions) -> Self {
        Self {
            ctx: opts.into(),
            tool_router: Self::tool_router(),
        }
    }
}

/// Map any `quay-core` error into an MCP tool error.
fn to_mcp_err(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

#[tool_router]
impl QuayServer {
    /// Search configured skill hubs for skills matching a query.
    #[tool(
        name = "quay_search",
        description = "Search configured skill hubs for skills matching a query.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    fn quay_search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<Json<SearchResults>, ErrorData> {
        let cfg = self
            .ctx
            .load_config_with(p.profile.as_deref())
            .map_err(to_mcp_err)?;
        let fetcher = self.ctx.fetcher();
        let filters = SearchFilters {
            query: &p.query,
            remote: p.remote.as_deref(),
            tag: p.tag.as_deref(),
        };
        let hits = search(&cfg, &fetcher, &filters).map_err(to_mcp_err)?;
        let results = hits
            .into_iter()
            .map(|h| SearchResultRow {
                name: h.name,
                version: h.version,
                remote: h.remote,
                description: h.description,
                category: h.category,
                tags: h.tags,
            })
            .collect();
        Ok(Json(SearchResults { results }))
    }

    /// Show details for one skill from the configured hubs.
    #[tool(
        name = "quay_info",
        description = "Show details for one skill from the configured hubs.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    fn quay_info(&self, Parameters(p): Parameters<SkillRef>) -> Result<Json<SkillInfo>, ErrorData> {
        let cfg = self
            .ctx
            .load_config_with(p.profile.as_deref())
            .map_err(to_mcp_err)?;
        let fetcher = self.ctx.fetcher();
        let mgr = SkillManager::new(&cfg, &fetcher, &fetcher, self.ctx.project.clone());
        let entry = mgr
            .info(&p.skill, p.remote.as_deref())
            .map_err(to_mcp_err)?;
        Ok(Json(SkillInfo {
            name: p.skill,
            version: entry.version,
            description: entry.description,
            category: entry.category,
            tags: entry.tags,
            files: entry.files,
        }))
    }

    /// List skills installed in this project's canonical skills directory.
    #[tool(
        name = "quay_list",
        description = "List skills installed in this project's canonical skills directory.",
        annotations(read_only_hint = true)
    )]
    fn quay_list(&self) -> Result<Json<InstalledSkills>, ErrorData> {
        let push_log = PushLog::default();
        let locals = scan_local(&self.ctx.project, &push_log);
        let skills = locals
            .iter()
            .map(|s| InstalledSkill {
                name: s.meta.name.clone(),
                path: s.canonical_path().display().to_string(),
            })
            .collect();
        Ok(Json(InstalledSkills { skills }))
    }

    /// Compare locally installed skills against the hubs; list upgrades.
    #[tool(
        name = "quay_outdated",
        description = "Compare locally installed skills against the hubs; list upgrades.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    fn quay_outdated(
        &self,
        Parameters(p): Parameters<OutdatedParams>,
    ) -> Result<Json<OutdatedReport>, ErrorData> {
        let cfg = self
            .ctx
            .load_config_with(p.profile.as_deref())
            .map_err(to_mcp_err)?;
        Ok(Json(self.outdated_report(&cfg).map_err(to_mcp_err)?))
    }

    /// Rows worth acting on: a higher version on the hub, or content that
    /// differs at an unchanged version. The latter is invisible to semver —
    /// bumping `version` on push is a convention quay does not enforce.
    fn outdated_report(&self, cfg: &quay_core::Config) -> quay_core::Result<OutdatedReport> {
        let fetcher = self.ctx.fetcher();
        let rows = outdated_for_local(&self.ctx.project, self.ctx.config_dir(), cfg, &fetcher)?;
        let outdated = rows
            .into_iter()
            .filter(|r| r.upgrade_available || r.content_drift)
            .map(|r| OutdatedRow {
                name: r.name,
                remote: r.remote,
                local_sha: r.local_sha,
                remote_sha: r.remote_sha,
                available: r.available,
                content_drift: r.content_drift,
            })
            .collect();
        Ok(OutdatedReport { outdated })
    }

    /// Test-only: run the outdated logic without the MCP wrapper.
    #[doc(hidden)]
    pub fn quay_outdated_for_test(&self) -> anyhow::Result<OutdatedReport> {
        let cfg = self.ctx.load_config_with(None)?;
        Ok(self.outdated_report(&cfg)?)
    }

    /// List all SKILL.md files found in this project (canonical + mirrors).
    #[tool(
        name = "quay_scan",
        description = "List all SKILL.md files found in this project (canonical + mirrors).",
        annotations(read_only_hint = true)
    )]
    fn quay_scan(&self) -> Result<Json<ScanReport>, ErrorData> {
        let push_log = PushLog::default();
        let locals = scan_local(&self.ctx.project, &push_log);
        let locations = locals
            .iter()
            .flat_map(|s| {
                s.locations.iter().map(move |loc| ScannedLocation {
                    name: s.meta.name.clone(),
                    path: loc.path.display().to_string(),
                    root: loc.root.label().to_string(),
                })
            })
            .collect();
        Ok(Json(ScanReport { locations }))
    }

    /// Validate an installed skill's SKILL.md frontmatter.
    #[tool(
        name = "quay_validate",
        description = "Validate an installed skill's SKILL.md frontmatter.",
        annotations(read_only_hint = true)
    )]
    fn quay_validate(
        &self,
        Parameters(p): Parameters<ValidateParams>,
    ) -> Result<Json<ValidateResult>, ErrorData> {
        let cfg = self.ctx.load_config().map_err(to_mcp_err)?;
        // NOTE: skill name is trusted (local MCP server, agent-supplied). A name with
        // path separators could escape the canonical dir; acceptable in this threat model.
        let md_path = self
            .ctx
            .project
            .join(&cfg.install.canonical)
            .join(&p.skill)
            .join("SKILL.md");
        let raw = std::fs::read_to_string(&md_path)
            .map_err(|e| to_mcp_err(format!("{}: {e}", md_path.display())))?;
        match parse_skill(&raw, &md_path.display().to_string()) {
            Ok(_) => Ok(Json(ValidateResult {
                ok: true,
                errors: vec![],
            })),
            Err(e) => Ok(Json(ValidateResult {
                ok: false,
                errors: vec![e.to_string()],
            })),
        }
    }

    /// Install a skill from a hub into this project's skills directory.
    #[tool(
        name = "quay_add",
        description = "Install a skill from a hub into this project's skills directory.",
        annotations(read_only_hint = false, open_world_hint = true)
    )]
    fn quay_add(
        &self,
        Parameters(p): Parameters<AddParams>,
    ) -> Result<Json<WriteResult>, ErrorData> {
        let r = self
            .add_inner(&p.skill, p.remote.as_deref(), p.force, p.profile.as_deref())
            .map_err(to_mcp_err)?;
        Ok(Json(r))
    }

    /// Mirror an installed skill into the configured agent directories.
    #[tool(
        name = "quay_link",
        description = "Mirror an installed skill into the configured agent directories (.claude, .codex, …).",
        annotations(read_only_hint = false)
    )]
    fn quay_link(
        &self,
        Parameters(p): Parameters<SkillName>,
    ) -> Result<Json<WriteResult>, ErrorData> {
        let cfg = self
            .ctx
            .load_config_with(p.profile.as_deref())
            .map_err(to_mcp_err)?;
        let actions = linker::apply_all(&cfg.install, &self.ctx.project, &p.skill, false)
            .map_err(to_mcp_err)?;
        Ok(Json(WriteResult {
            ok: true,
            message: format!("mirrored {} ({} target(s))", p.skill, actions.len()),
        }))
    }

    /// Update an installed skill to the latest version from its hub.
    #[tool(
        name = "quay_update",
        description = "Update an installed skill to the latest version from its hub.",
        annotations(read_only_hint = false, open_world_hint = true)
    )]
    fn quay_update(
        &self,
        Parameters(p): Parameters<SkillName>,
    ) -> Result<Json<WriteResult>, ErrorData> {
        let cfg = self
            .ctx
            .load_config_with(p.profile.as_deref())
            .map_err(to_mcp_err)?;
        let fetcher = self.ctx.fetcher();
        let mgr = SkillManager::new(&cfg, &fetcher, &fetcher, self.ctx.project.clone());
        mgr.update_one(&p.skill).map_err(to_mcp_err)?;
        Ok(Json(WriteResult {
            ok: true,
            message: format!("updated {}", p.skill),
        }))
    }

    /// Uninstall a skill from this project's skills directory.
    #[tool(
        name = "quay_remove",
        description = "Uninstall a skill from this project's skills directory.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    fn quay_remove(
        &self,
        Parameters(p): Parameters<SkillName>,
    ) -> Result<Json<WriteResult>, ErrorData> {
        let cfg = self
            .ctx
            .load_config_with(p.profile.as_deref())
            .map_err(to_mcp_err)?;
        let fetcher = self.ctx.fetcher();
        let mgr = SkillManager::new(&cfg, &fetcher, &fetcher, self.ctx.project.clone());
        mgr.remove(&p.skill).map_err(to_mcp_err)?;
        Ok(Json(WriteResult {
            ok: true,
            message: format!("removed {}", p.skill),
        }))
    }

    /// Publish a local skill to its hub by opening a PR (or pushing directly).
    #[tool(
        name = "quay_push",
        description = "Publish a local skill to its hub by opening a pull request (or pushing directly, per remote config).",
        annotations(read_only_hint = false, open_world_hint = true)
    )]
    fn quay_push(
        &self,
        Parameters(p): Parameters<PushParams>,
    ) -> Result<Json<PushOutcome>, ErrorData> {
        use quay_core::BumpKind;
        let bump = match p.bump.as_deref() {
            None | Some("patch") => BumpKind::Patch,
            Some("minor") => BumpKind::Minor,
            Some("major") => BumpKind::Major,
            Some(other) => {
                return Err(to_mcp_err(format!(
                    "invalid bump: {other} (expected patch|minor|major)"
                )))
            }
        };
        let outcome = self
            .run_push(&p.skill, p.remote.as_deref(), bump, p.profile.as_deref())
            .map_err(to_mcp_err)?;
        Ok(Json(outcome))
    }

    /// Add a hub remote to this project's quay config.
    #[tool(
        name = "quay_remote",
        description = "Add a hub remote to this project's quay config.",
        annotations(read_only_hint = false, open_world_hint = true)
    )]
    fn quay_remote(
        &self,
        Parameters(p): Parameters<RemoteAddParams>,
    ) -> Result<Json<WriteResult>, ErrorData> {
        self.add_remote(&p.name, &p.url, p.default)
            .map_err(to_mcp_err)?;
        Ok(Json(WriteResult {
            ok: true,
            message: format!("added remote {} -> {}", p.name, p.url),
        }))
    }
}

/// Shared logic and test-only helpers that run tool logic without the MCP wrapper.
impl QuayServer {
    /// Core add logic, shared by the `quay_add` tool and the test helper.
    fn add_inner(
        &self,
        skill: &str,
        remote: Option<&str>,
        force: bool,
        profile: Option<&str>,
    ) -> anyhow::Result<crate::params::WriteResult> {
        let cfg = self.ctx.load_config_with(profile)?;
        let fetcher = self.ctx.fetcher();
        let mgr = SkillManager::new(&cfg, &fetcher, &fetcher, self.ctx.project.clone());
        mgr.add_with_force(skill, remote, force)?;
        Ok(crate::params::WriteResult {
            ok: true,
            message: format!("installed {skill}"),
        })
    }

    /// Test-only: run the add logic without the MCP wrapper.
    #[doc(hidden)]
    pub fn quay_add_for_test(
        &self,
        skill: &str,
        remote: Option<&str>,
        force: bool,
    ) -> anyhow::Result<crate::params::WriteResult> {
        self.add_inner(skill, remote, force, None)
    }

    /// Core remote-add logic, ported from `quay-cli`'s `commands::remote::run`
    /// Add path MINUS all printing: read the project config, insert a remote
    /// with no explicit provider, write it back.
    fn add_remote(&self, name: &str, url: &str, default: bool) -> anyhow::Result<()> {
        use quay_core::{Config, ProviderKind, QuayError, RemoteConfig};
        // Mirror `commands::remote::run`'s Add path: read the project config
        // file, insert a new remote, write it back. The project config uses the
        // same `Config` shape (flat `[remotes.*]`), so `Config::read`/`write`
        // round-trip it.
        let project_config = self.ctx.project.join(".quay/config.toml");
        let mut cfg = Config::read(&project_config)?;
        if cfg.remotes.contains_key(name) {
            return Err(QuayError::RemoteExists(name.to_string()).into());
        }
        if default {
            for r in cfg.remotes.values_mut() {
                r.default = false;
            }
        }
        // No explicit provider — stored as `None`. Provider is resolved from
        // the URL at push time by `provider_for_remote`. The MCP surface does
        // not expose a `--provider` override (unlike the CLI).
        let kind: Option<ProviderKind> = None;
        cfg.remotes.insert(
            name.to_string(),
            RemoteConfig {
                url: url.to_string(),
                default,
                provider: kind,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        cfg.write(&project_config)?;
        Ok(())
    }

    /// Core push logic, ported from `quay-cli`'s `commands::push::push_skill`
    /// MINUS all printing.
    fn run_push(
        &self,
        skill: &str,
        remote: Option<&str>,
        bump: quay_core::BumpKind,
        profile: Option<&str>,
    ) -> anyhow::Result<crate::params::PushOutcome> {
        use quay_core::{GitShellClient, QuayError, SkillPusher};

        let project = &self.ctx.project;
        let cfg = self.ctx.load_config_with(profile)?;
        if cfg.remotes.is_empty() {
            return Err(QuayError::ConfigValidation(
                "no remotes configured — add a remote with quay_remote first".into(),
            )
            .into());
        }

        let git = GitShellClient;

        // Resolve the target remote URL + provider so we can select the right
        // opener, mirroring push_skill's remote-resolution logic.
        let (remote_url, remote_provider) = {
            let remote_name = match remote {
                Some(name) => name.to_string(),
                None => cfg
                    .default_remote()
                    .map(|(n, _)| n.clone())
                    .ok_or_else(|| {
                        QuayError::ConfigValidation("no default remote — pass remote=<name>".into())
                    })?,
            };
            let r = cfg
                .remotes
                .get(&remote_name)
                .ok_or_else(|| QuayError::RemoteUnknown(remote_name.clone()))?;
            (r.url.clone(), r.provider)
        };
        // `provider_for_remote` returns a `Box<dyn Provider>` which implements
        // `PrOpener` — the genuine opener that opens a real PR / direct push.
        let opener = quay_core::provider_for_remote(&remote_url, remote_provider);

        let clone_root = std::env::temp_dir().join(format!("quay-push-{}", std::process::id()));
        std::fs::create_dir_all(&clone_root).map_err(|source| QuayError::Io {
            path: clone_root.display().to_string(),
            source,
        })?;

        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.clone(),
            config_dir: self
                .ctx
                .user_config
                .as_deref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf()),
            author: None,
        };
        // No push_mode / direct_branch override — honour the remote config.
        let result = pusher.push(skill, remote, bump, &clone_root, None, None)?;

        // Best-effort cleanup of the temp clone tree.
        let _ = std::fs::remove_dir_all(&clone_root);

        // Map PushResult → PushOutcome. PR-mode pushes carry a PrInfo with a
        // url; direct-mode pushes have `pr == None`, so url is None and the
        // summary describes the branch + commit.
        let outcome = match &result.pr {
            Some(pr) => crate::params::PushOutcome {
                ok: true,
                url: Some(pr.url.clone()),
                message: format!(
                    "pushed {skill} v{} to {} (branch {})",
                    result.version, result.remote, result.branch
                ),
            },
            None => {
                let short_sha: String = result.commit_sha.chars().take(8).collect();
                crate::params::PushOutcome {
                    ok: true,
                    url: None,
                    message: format!(
                        "pushed direct: {skill} v{} -> {} (branch {} at {short_sha})",
                        result.version, result.remote, result.branch
                    ),
                }
            }
        };
        Ok(outcome)
    }

    /// Test-only: run the remote-add logic without the MCP wrapper.
    #[doc(hidden)]
    pub fn add_remote_for_test(&self, name: &str, url: &str, default: bool) -> anyhow::Result<()> {
        self.add_remote(name, url, default)
    }

    /// Test-only: run the push logic without the MCP wrapper.
    #[doc(hidden)]
    pub fn run_push_for_test(
        &self,
        skill: &str,
        remote: Option<&str>,
        bump: quay_core::BumpKind,
    ) -> anyhow::Result<crate::params::PushOutcome> {
        self.run_push(skill, remote, bump, None)
    }
}

// `name` brands the `initialize` response's `serverInfo`. Without it, rmcp's
// default `get_info()` reports the rmcp crate's identity ("rmcp"). Omitting
// `version` makes the macro default it to this crate's `CARGO_PKG_VERSION`
// (see rmcp-macros `build_get_info`). The macro still generates `get_info()`
// with the tools capability + default protocol version preserved.
#[tool_handler(name = "quay-mcp")]
impl ServerHandler for QuayServer {}

/// Serve over stdio until the client disconnects.
pub async fn serve(opts: ServeOptions) -> anyhow::Result<()> {
    let server = QuayServer::new(opts);
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
