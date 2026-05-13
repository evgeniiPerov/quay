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
        Command::Add {
            skill,
            remote,
            force,
            interactive,
        } => {
            use commands::interactive::should_auto_interactive;
            if should_auto_interactive(skill.is_some(), interactive) {
                commands::add::run_interactive(
                    remote.as_deref(),
                    force,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else if let Some(skill) = skill {
                commands::add::run(
                    &skill,
                    remote.as_deref(),
                    force,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else {
                return Err(
                    "skill name required, or pass -i in a terminal\n       use `quay add --help` for usage".into(),
                );
            }
        }
        Command::List => commands::list::run(&project, cli.json)?,
        Command::Remove {
            skill,
            everywhere,
            interactive,
        } => {
            use commands::interactive::should_auto_interactive;
            if should_auto_interactive(skill.is_some(), interactive) {
                commands::remove::run_interactive(
                    everywhere,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else if let Some(skill) = skill {
                commands::remove::run(
                    &skill,
                    everywhere,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else {
                return Err(
                    "skill name required, or pass -i in a terminal\n       use `quay remove --help` for usage".into(),
                );
            }
        }
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
        Command::Update {
            skill,
            dry_run,
            interactive,
            all,
        } => {
            use commands::interactive::is_tty;
            if all {
                // Explicit bypass: update everything without the picker.
                commands::update::run(
                    None,
                    dry_run,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else if interactive {
                commands::update::run_interactive(
                    dry_run,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else if let Some(skill) = skill {
                commands::update::run(
                    Some(skill.as_str()),
                    dry_run,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else {
                // Bare invocation: TTY → picker, non-TTY → update all.
                if is_tty() {
                    commands::update::run_interactive(
                        dry_run,
                        cli.profile.as_deref(),
                        &project,
                        user_config.as_deref(),
                        cli.json,
                    )?;
                } else {
                    commands::update::run(
                        None,
                        dry_run,
                        cli.profile.as_deref(),
                        &project,
                        user_config.as_deref(),
                        cli.json,
                    )?;
                }
            }
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
            direct_branch,
            interactive,
        } => {
            use commands::interactive::should_auto_interactive;
            // Empty string on CLI means "no override" (same as unset).
            let direct_branch_ref: Option<&str> =
                direct_branch.as_deref().filter(|s| !s.is_empty());
            if should_auto_interactive(skill.is_some(), interactive) {
                commands::push::run_interactive(
                    remote.as_deref(),
                    bump,
                    push_mode.map(quay_core::config::PushMode::from),
                    direct_branch_ref,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else if let Some(skill) = skill {
                commands::push::run(
                    &skill,
                    remote.as_deref(),
                    bump,
                    push_mode.map(quay_core::config::PushMode::from),
                    direct_branch_ref,
                    cli.profile.as_deref(),
                    &project,
                    user_config.as_deref(),
                    cli.json,
                )?;
            } else {
                return Err(
                    "skill name required, or pass -i in a terminal\n       use `quay push --help` for usage".into(),
                );
            }
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
    config_io::default_config_dir().map(|d| d.join("config.toml"))
}
