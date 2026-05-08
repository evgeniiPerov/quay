use clap::{Parser, Subcommand};
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
    Add {
        skill: String,
        #[arg(long)]
        remote: Option<String>,
    },
    /// List installed skills
    List,
    /// Remove a previously installed skill
    Remove { skill: String },
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
        skill: Option<String>,
        /// Show what would change without writing to disk.
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply the lockfile exactly — refetch any missing or drifted files at the recorded sha.
    Sync,
    /// Scaffold a new local SKILL.md in .agents/skills/<name>/
    Create {
        /// Skill name (kebab-case recommended).
        name: String,
        /// Override the auto-detected author email.
        #[arg(long)]
        author: Option<String>,
    },
    /// Validate a local skill's frontmatter (offline, no network)
    Validate { skill: String },
    /// Push a local skill to a hub via PR
    Push {
        skill: String,
        #[arg(long)]
        remote: Option<String>,
        /// Bump kind: patch | minor | major | as-written. Default: as-written.
        #[arg(long, value_parser = parse_bump, default_value = "as-written")]
        bump: BumpArg,
    },
    /// Manage user profiles (multi-org identities + remote bundles)
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
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

#[derive(Subcommand, Debug)]
pub enum ProfileAction {
    /// List all profiles, marking the active one.
    List,
    /// Print the active profile name.
    Current,
    /// Add a new profile.
    Add {
        name: String,
        /// Author email for commits made under this profile.
        #[arg(long)]
        email: Option<String>,
        /// Optionally seed the profile with a first remote: `--remote=<name>=<url>`.
        #[arg(long)]
        remote: Option<String>,
        /// Mark this profile as the new `active_profile`.
        #[arg(long)]
        activate: bool,
    },
    /// Set the active profile.
    Use { name: String },
    /// Remove a profile (cannot remove the last one).
    Remove { name: String },
    /// Print full profile contents.
    Show { name: Option<String> },
    /// Rename a profile.
    Rename { old: String, new: String },
}

#[derive(Subcommand, Debug)]
pub enum RemoteAction {
    /// Add a new remote
    Add {
        name: String,
        url: String,
        #[arg(long)]
        default: bool,
    },
    /// List configured remotes
    List,
    /// Remove a remote
    Remove { name: String },
}
