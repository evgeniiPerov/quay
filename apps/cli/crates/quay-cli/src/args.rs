use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "quay", version, about = "Skill registry CLI")]
pub struct Cli {
    /// Project root (defaults to current directory)
    #[arg(long, global = true)]
    pub project: Option<PathBuf>,

    /// Override user config path (defaults to ~/.config/quay/config.toml)
    #[arg(long, global = true)]
    pub user_config: Option<PathBuf>,

    /// Override the active profile for this invocation.
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Output JSON instead of human-readable text
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize quay in the current project
    Init,
    /// Manage configured hub remotes
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
    /// Install a skill from a configured remote
    #[command(visible_alias = "ls")]
    Add {
        /// Skill name(s) to install. Omit when using --interactive (-i).
        #[arg(conflicts_with = "interactive")]
        skill: Option<String>,
        #[arg(long)]
        remote: Option<String>,
        /// Overwrite the skill if it already exists locally.
        #[arg(long)]
        force: bool,
        /// Open an interactive checkbox list to pick skills to install.
        /// Mutually exclusive with the positional skill argument.
        #[arg(short = 'i', long)]
        interactive: bool,
        /// Suppress the diff body on a collision (still prints the verdict line).
        #[arg(long)]
        no_diff: bool,
    },
    /// List installed skills
    List,
    /// Remove a skill — locally by default, from the hub with --remote, or both with --everywhere
    Remove {
        /// Skill name to remove. Omit when using --interactive (-i) or bare --remote.
        #[arg(conflicts_with = "interactive")]
        skill: Option<String>,
        /// Remove only from the hub (default remote), keeping the local copy.
        #[arg(long, conflicts_with = "everywhere")]
        remote: bool,
        /// Remove both locally and from the hub (default remote).
        #[arg(long)]
        everywhere: bool,
        /// Open an interactive checkbox list to pick skills to remove.
        /// Mutually exclusive with the positional skill argument.
        #[arg(short = 'i', long)]
        interactive: bool,
    },
    /// Show metadata for a skill (without installing)
    Info {
        skill: String,
        #[arg(long)]
        remote: Option<String>,
    },
    /// Search across configured remotes
    Search {
        /// Free-text query matched against skill name + description (case-insensitive)
        query: String,
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        tag: Option<String>,
    },
    /// List installed skills that have newer versions available
    Outdated,
    /// Update installed skills to the latest available version
    Update {
        /// Update only this skill; if omitted, updates every installed skill.
        #[arg(conflicts_with_all = ["interactive", "all"])]
        skill: Option<String>,
        /// Show what would change without writing to disk.
        #[arg(long)]
        dry_run: bool,
        /// Open an interactive checkbox list of outdated skills to update.
        /// Mutually exclusive with the positional skill argument.
        #[arg(short = 'i', long, conflicts_with = "all")]
        interactive: bool,
        /// Update every installed skill without opening the picker, even in a terminal.
        /// Explicit bypass for the TTY auto-trigger.
        #[arg(long, conflicts_with = "interactive")]
        all: bool,
    },
    /// Discover local skills under `.agents/skills/` and report their sync status.
    Scan {
        /// Override the install canonical root (default: `<project>/.agents/skills`).
        #[arg(long)]
        root: Option<std::path::PathBuf>,
        /// Emit JSON instead of a human table.
        #[arg(long)]
        json: bool,
    },
    /// Generate or verify `skills-lock.json` (vercel-compatible lockfile).
    Lock {
        /// Report drift against the lockfile and exit non-zero if any.
        #[arg(long, conflicts_with_all = ["heal", "sync"])]
        check: bool,
        /// Rewrite the lockfile to match what is on disk (idempotent).
        #[arg(long, conflicts_with_all = ["check", "sync"])]
        heal: bool,
        /// Install locked skills that are missing on disk.
        #[arg(long, conflicts_with_all = ["check", "heal"])]
        sync: bool,
        /// With --check, also probe whether each source is reachable. [not yet implemented]
        #[arg(long, requires = "check", conflicts_with_all = ["heal", "sync"])]
        online: bool,
    },
    /// Validate a local skill's frontmatter (offline, no network).
    Validate {
        skill: String,
        /// Treat missing frontmatter or required fields as errors (exit 1).
        /// Default: prints warnings to stderr, exits 0 (lenient/soft mode).
        #[arg(long)]
        strict: bool,
    },
    /// Push a local skill to a hub via PR (or directly, if --push-mode=direct
    /// or the remote's TOML says so).
    Push {
        /// Skill name to push. Omit when using --interactive (-i).
        #[arg(conflicts_with = "interactive")]
        skill: Option<String>,
        #[arg(long)]
        remote: Option<String>,
        /// Bump kind: patch | minor | major | as-written. Default: as-written.
        #[arg(long, value_parser = parse_bump, default_value = "as-written")]
        bump: BumpArg,
        /// Override the remote's `push_mode` setting for this invocation.
        /// Values: pr, direct.
        #[arg(long, value_enum)]
        push_mode: Option<PushModeArg>,
        /// Override the target branch for direct-mode pushes for this invocation.
        /// Ignored when push mode resolves to `pr`.
        /// Pass an empty string to explicitly target the default branch.
        #[arg(long)]
        direct_branch: Option<String>,
        /// Open an interactive checkbox list of local skills to push.
        /// Mutually exclusive with the positional skill argument.
        #[arg(short = 'i', long)]
        interactive: bool,
    },
    /// Manage user profiles (multi-org identities + remote bundles)
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Rebuild a hub's `registry.json` from disk truth and push it back.
    ///
    /// Clones the remote, walks `skills/<name>/SKILL.md`, regenerates
    /// `registry.json` containing every discovered skill, then commits and
    /// pushes (PR or direct per the remote's `push_mode`).
    RebuildRegistry {
        /// Remote name. Uses the default remote when omitted.
        remote: Option<String>,
        /// Override the remote's `push_mode` for this invocation.
        #[arg(long, value_enum)]
        push_mode: Option<PushModeArg>,
    },
    /// Apply or verify mirrors from `[install].mirrors` config.
    Link {
        #[command(subcommand)]
        action: Option<LinkAction>,
        /// Overwrite existing entries even if they conflict with quay's expected layout.
        #[arg(long, global = true)]
        force: bool,
    },
    /// Launch the interactive TUI.
    Tui {
        /// Probe the config and exit without launching the TUI.
        /// Exit code 0: onboarding not needed. Exit code 2: onboarding needed.
        #[arg(long, hide = true)]
        check_config_only: bool,
    },
    /// Run the MCP server (for AI agents / MCP clients). Speaks MCP over stdio.
    #[command(hide = true)]
    Mcp {
        #[command(subcommand)]
        action: Option<McpAction>,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum McpAction {
    /// Write MCP registration config for a specific client.
    Install {
        /// Target client: claude, codex, or cursor.
        client: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum LinkAction {
    /// Verify mirrors are intact; exit non-zero if drifted.
    Check,
    /// Add a new mirror to the project config and apply it for every installed skill.
    Add {
        path: PathBuf,
        #[arg(long, default_value = "auto")]
        strategy: String,
    },
    /// Remove a mirror from the project config (does not delete the mirror dir).
    Remove { path: PathBuf },
}

/// Version bump strategy for `quay push`.
#[derive(Debug, Clone, Copy)]
pub enum BumpArg {
    Patch,
    Minor,
    Major,
    AsWritten,
}

fn parse_bump(s: &str) -> std::result::Result<BumpArg, String> {
    match s {
        "patch" => Ok(BumpArg::Patch),
        "minor" => Ok(BumpArg::Minor),
        "major" => Ok(BumpArg::Major),
        "as-written" => Ok(BumpArg::AsWritten),
        other => Err(format!(
            "invalid --bump '{}': expected patch|minor|major|as-written",
            other
        )),
    }
}

/// Push-mode override for `quay push --push-mode`.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum PushModeArg {
    Pr,
    Direct,
}

impl From<PushModeArg> for quay_core::config::PushMode {
    fn from(p: PushModeArg) -> Self {
        match p {
            PushModeArg::Pr => quay_core::config::PushMode::Pr,
            PushModeArg::Direct => quay_core::config::PushMode::Direct,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum ProfileAction {
    /// List all profiles, marking the active one.
    List,
    /// Print the active profile name.
    Current,
    /// Add a new profile.
    ///
    /// Three mutually-exclusive modes:
    ///   - Explicit flags:  quay profile add <name> --email <e> --remote n=url ...
    ///   - Wizard:          quay profile add -i           (recommended first time)
    ///   - TOML ingestion:  quay profile add <name> --from-toml <path|->
    ///
    /// EXAMPLES
    ///
    /// Single remote, GitHub PR mode (provider auto-detected from URL):
    ///   quay profile add work --email me@work.com \
    ///     --remote gh=git@github.com:org/skills.git --activate
    ///
    /// Azure DevOps, direct push to non-default branch (e.g. develop):
    ///   quay profile add team --email me@team.com \
    ///     --remote hub=https://dev.azure.com/org/proj/_git/repo \
    ///     --provider azuredevops --push-mode direct --direct-branch develop
    ///
    /// Multiple remotes — each --provider / --push-mode / --direct-branch /
    /// --default applies to the most recently specified --remote:
    ///   quay profile add multi --email me@x.com \
    ///     --remote a=git@github.com:org/a.git --default \
    ///     --remote b=git@gitlab.com:org/b.git --push-mode direct --direct-branch main
    ///
    /// PowerShell note: URLs with `&` or `?` must be single-quoted; backtick (`)
    /// at end of line continues the command on the next line.
    #[command(verbatim_doc_comment)]
    Add {
        /// Profile name (required unless `-i` / `--interactive` is used).
        #[arg(
            conflicts_with = "interactive",
            required_unless_present = "interactive"
        )]
        name: Option<String>,
        /// Run the multi-step interactive wizard. Mutually exclusive with
        /// `--email`, `--remote`, and `--from-toml`.
        #[arg(
            short = 'i',
            long,
            conflicts_with_all = ["email", "remote", "from_toml"]
        )]
        interactive: bool,
        /// Read profile config from a TOML file or `-` for stdin.
        /// Mutually exclusive with `-i`, `--email`, and `--remote`.
        #[arg(
            long,
            value_name = "PATH|-",
            conflicts_with_all = ["interactive", "email", "remote"]
        )]
        from_toml: Option<String>,
        /// Author email for commits made under this profile.
        /// Mutually exclusive with `-i` and `--from-toml`.
        #[arg(long, conflicts_with_all = ["interactive", "from_toml"])]
        email: Option<String>,
        /// Seed a remote: `--remote <name>=<url>` (repeatable).
        /// All following `--provider`, `--push-mode`, `--direct-branch`,
        /// `--default` flags apply to THIS remote until the next `--remote`.
        /// Mutually exclusive with `-i` and `--from-toml`.
        #[arg(
            long,
            value_name = "NAME=URL",
            action = clap::ArgAction::Append,
            conflicts_with_all = ["interactive", "from_toml"]
        )]
        remote: Vec<String>,
        /// Provider for the most recently specified `--remote`. When omitted,
        /// the provider is auto-detected from the URL (github / gitlab /
        /// bitbucket / azuredevops / github-enterprise).
        #[arg(long, value_enum, conflicts_with_all = ["interactive", "from_toml"])]
        provider: Vec<ProviderKindArg>,
        /// Push mode for the most recently specified `--remote`:
        ///   `pr`     — open a pull request (default)
        ///   `direct` — git push directly to a branch (see `--direct-branch`)
        #[arg(long, value_enum, action = clap::ArgAction::Append, conflicts_with_all = ["interactive", "from_toml"])]
        push_mode: Vec<PushModeArg>,
        /// Branch for direct-mode pushes. Applies to the most recently
        /// specified `--remote`. Omit to push to the hub's default branch
        /// (e.g. `main`/`master`). Use this when your team merges to a
        /// non-default integration branch like `develop` or `staging`.
        /// Repeatable; positional per `--remote`.
        #[arg(long, value_name = "BRANCH", action = clap::ArgAction::Append, conflicts_with_all = ["interactive", "from_toml"])]
        direct_branch: Vec<String>,
        /// Mark the most recently specified `--remote` as the default remote
        /// for this profile. Only one remote should be default per profile.
        /// Repeat once per `--remote` that should be the default.
        #[arg(long, action = clap::ArgAction::Count, conflicts_with_all = ["interactive", "from_toml"])]
        default: u8,
        /// Set this profile as the new `active_profile`.
        #[arg(long)]
        activate: bool,
    },
    /// Set the active profile.
    Use {
        /// Profile name to activate. Omit when using --interactive (-i).
        #[arg(conflicts_with = "interactive")]
        name: Option<String>,
        /// Open an interactive single-select list of profiles to choose from.
        /// Mutually exclusive with the positional name argument.
        #[arg(short = 'i', long)]
        interactive: bool,
    },
    /// Remove a profile (cannot remove the last one).
    Remove { name: String },
    /// Print full profile contents.
    Show { name: Option<String> },
    /// Rename a profile.
    Rename { old: String, new: String },
    /// Edit an existing profile.
    ///
    /// Three mutually-exclusive modes:
    ///   * Explicit flags — `quay profile edit <name> --email <e>`
    ///   * Wizard — `quay profile edit <name> -i` (or `-i` alone to pick a profile first)
    ///   * TOML ingestion — `quay profile edit <name> --from-toml <path|->`
    Edit {
        /// Profile name to edit.
        /// When `-i` is used without a name, an interactive picker opens first.
        #[arg(required_unless_present = "interactive")]
        name: Option<String>,
        /// Run the multi-step interactive wizard pre-populated with current
        /// values. Mutually exclusive with `--email` and `--from-toml`.
        #[arg(
            short = 'i',
            long,
            conflicts_with_all = ["email", "from_toml"]
        )]
        interactive: bool,
        /// Replace the entire profile content from a TOML file or `-` for stdin.
        /// Mutually exclusive with `-i` and `--email`.
        #[arg(
            long,
            value_name = "PATH|-",
            conflicts_with_all = ["interactive", "email"]
        )]
        from_toml: Option<String>,
        /// New author email for this profile.
        /// Mutually exclusive with `-i` and `--from-toml`.
        #[arg(long, conflicts_with_all = ["interactive", "from_toml"])]
        email: Option<String>,
    },
}

/// Provider kind for explicit remote provider override.
#[derive(Clone, Copy, Debug, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ProviderKindArg {
    Github,
    Githubenterprise,
    Gitlab,
    Bitbucket,
    Azuredevops,
}

impl From<ProviderKindArg> for quay_core::ProviderKind {
    fn from(a: ProviderKindArg) -> Self {
        use quay_core::ProviderKind as K;
        match a {
            ProviderKindArg::Github => K::GitHub,
            ProviderKindArg::Githubenterprise => K::GitHubEnterprise,
            ProviderKindArg::Gitlab => K::GitLab,
            ProviderKindArg::Bitbucket => K::Bitbucket,
            ProviderKindArg::Azuredevops => K::AzureDevOps,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum RemoteAction {
    /// Add a new remote
    Add {
        name: String,
        url: String,
        #[arg(long)]
        default: bool,
        /// Explicitly set the provider kind (auto-detected from URL if omitted).
        #[arg(long, value_enum)]
        provider: Option<ProviderKindArg>,
        /// Push mode for this remote: pr (default) or direct.
        #[arg(long, value_enum)]
        push_mode: Option<PushModeArg>,
        /// Target branch for direct-mode pushes on this remote.
        /// Omit to push to the hub's default branch.
        #[arg(long)]
        direct_branch: Option<String>,
    },
    /// Test connectivity to a configured remote
    Test {
        /// Name of the remote to test
        name: String,
    },
    /// List configured remotes
    List,
    /// Remove a remote
    Remove { name: String },
    /// Edit an existing remote
    Edit {
        /// Name of the remote to edit.
        name: String,
        /// New Git URL for the remote.
        #[arg(long)]
        url: Option<String>,
        /// New provider kind for the remote.
        #[arg(long, value_enum)]
        provider: Option<ProviderKindArg>,
        /// New push mode for the remote (`pr` or `direct`).
        #[arg(long, value_enum)]
        push_mode: Option<PushModeArg>,
        /// Target branch for direct-mode pushes on this remote.
        /// Pass an empty string (`--direct-branch ""`) to clear/unset the value.
        #[arg(long)]
        direct_branch: Option<String>,
        /// Mark this remote as the default (clears the flag on the previous default).
        #[arg(long)]
        default: bool,
    },
}
