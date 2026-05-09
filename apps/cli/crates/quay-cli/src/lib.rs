//! Quay CLI commands.

pub mod args;
pub mod commands;
pub mod config_io;
pub mod tui;
pub mod url_opener;

use args::{Cli, Command};
use clap::Parser;

pub fn run() -> i32 {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let project = cli
        .project
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let user_config = cli.user_config.clone().or_else(default_user_config_path);

    match cli.command {
        Command::Init => commands::init::run(&project, cli.json)?,
        Command::Remote { action } => commands::remote::run(
            action,
            &project,
            user_config.as_deref(),
            cli.profile.as_deref(),
            cli.json,
        )?,
        Command::Add { skill, remote } => {
            commands::add::run(
                &skill,
                remote.as_deref(),
                cli.profile.as_deref(),
                &project,
                user_config.as_deref(),
                cli.json,
            )?;
        }
        Command::List => commands::list::run(&project, cli.json)?,
        Command::Remove { skill } => commands::remove::run(
            &skill,
            cli.profile.as_deref(),
            &project,
            user_config.as_deref(),
            cli.json,
        )?,
        Command::Info { skill, remote } => {
            commands::info::run(
                &skill,
                remote.as_deref(),
                cli.profile.as_deref(),
                &project,
                user_config.as_deref(),
                cli.json,
            )?;
        }
        Command::Search { query, remote, tag } => {
            commands::search::run(
                &query,
                remote.as_deref(),
                tag.as_deref(),
                cli.profile.as_deref(),
                &project,
                user_config.as_deref(),
                cli.json,
            )?;
        }
        Command::Outdated => commands::outdated::run(
            &project,
            cli.profile.as_deref(),
            user_config.as_deref(),
            cli.json,
        )?,
        Command::Update { skill, dry_run } => {
            commands::update::run(
                skill.as_deref(),
                dry_run,
                cli.profile.as_deref(),
                &project,
                user_config.as_deref(),
                cli.json,
            )?;
        }
        Command::Sync => commands::sync::run(
            &project,
            cli.profile.as_deref(),
            user_config.as_deref(),
            cli.json,
        )?,
        Command::Create { name, author } => {
            commands::create::run(
                &name,
                author.as_deref(),
                cli.profile.as_deref(),
                &project,
                user_config.as_deref(),
                cli.json,
            )?;
        }
        Command::Scan { root, json } => {
            commands::scan::run(&project, root, json || cli.json)?;
        }
        Command::Validate { skill, strict } => {
            commands::validate::run(&skill, &project, cli.json, strict)?;
        }
        Command::Push {
            skill,
            remote,
            bump,
            push_mode,
        } => {
            commands::push::run(
                &skill,
                remote.as_deref(),
                bump,
                push_mode.map(quay_core::config::PushMode::from),
                cli.profile.as_deref(),
                &project,
                user_config.as_deref(),
                cli.json,
            )?;
        }
        Command::Profile { action } => {
            commands::profile::run(action, &project, user_config.as_deref(), cli.json)?;
        }
        Command::RebuildRegistry { remote, push_mode } => {
            commands::rebuild_registry::run(
                remote.as_deref(),
                push_mode.map(quay_core::config::PushMode::from),
                &project,
                user_config.as_deref(),
                cli.profile.as_deref(),
                cli.json,
            )?;
        }
        Command::Link { action, force } => {
            commands::link::run(action, force, &project, user_config.as_deref(), cli.json)?;
        }
        Command::Tui { check_config_only } => {
            commands::tui::run(
                &project,
                user_config.as_deref(),
                cli.profile.as_deref(),
                check_config_only,
            )?;
        }
    }
    Ok(())
}

fn default_user_config_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config/quay/config.toml"))
}
