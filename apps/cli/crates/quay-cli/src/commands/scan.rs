//! `quay scan` — list local skills and their sync status.

use quay_core::push_log::PushLog;
use quay_core::scanner::{scan_local, LocalSkill, ScanStatus, SkillFormat};
use std::path::{Path, PathBuf};

pub fn run(
    project_root: &Path,
    _root: Option<PathBuf>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Derive the user-level config dir from the standard default location,
    // respecting XDG_CONFIG_HOME / HOME.  A missing config dir is non-fatal.
    let config_dir = crate::config_io::default_config_dir();
    let push_log = PushLog::load(
        config_dir.as_deref().unwrap_or(project_root),
        Some(project_root),
    )
    .unwrap_or_default();
    let skills = scan_local(project_root, &push_log);

    if json {
        match serde_json::to_string_pretty(&skills_for_json(&skills)) {
            Ok(s) => {
                println!("{s}");
            }
            Err(e) => {
                eprintln!("scan: failed to serialise: {e}");
            }
        }
    } else {
        print_table(&skills);
    }

    Ok(())
}

fn print_table(skills: &[LocalSkill]) {
    if skills.is_empty() {
        println!("(no local skills found in any mirror root)");
        return;
    }
    println!(
        "{:<32}  {:<14}  {:<28}  {:<18}  PATH",
        "NAME", "FORMAT", "STATUS", "MIRRORS"
    );
    for s in skills {
        let format = match s.meta.format {
            SkillFormat::Frontmatter => "frontmatter",
            SkillFormat::SlashCommand => "slash-command",
            SkillFormat::Freestyle => "freestyle",
        };
        let status = match &s.status {
            ScanStatus::Local => "local".to_string(),
            ScanStatus::Installed { version, .. } => format!("installed v{version}"),
            ScanStatus::InstalledModified { version, .. } => {
                format!("installed-modified v{version}")
            }
            ScanStatus::PushedLocal {
                pr_url, commit_sha, ..
            } if pr_url.is_empty() => {
                let short: String = commit_sha.chars().take(8).collect();
                if short.is_empty() {
                    "pushed-direct".to_string()
                } else {
                    format!("pushed-direct ({short})")
                }
            }
            ScanStatus::PushedLocal { pr_url, .. } => format!("pushed-local ({pr_url})"),
        };
        let mirrors: Vec<&str> = s.locations.iter().map(|l| l.root.label()).collect();
        let drift = if s.has_drift() { " [drift]" } else { "" };
        println!(
            "{:<32}  {:<14}  {:<28}  {:<18}  {}{}",
            s.meta.name,
            format,
            status,
            mirrors.join(","),
            s.canonical_path().display(),
            drift,
        );
    }
}

fn skills_for_json(skills: &[LocalSkill]) -> serde_json::Value {
    let arr: Vec<_> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.meta.name,
                "description": s.meta.description,
                "version": s.meta.version,
                "tags": s.meta.tags,
                "format": s.meta.format,
                "path": s.canonical_path().display().to_string(),
                "sha256": s.canonical_sha256(),
                "mirrors": s.locations.iter().map(|l| l.root.label()).collect::<Vec<_>>(),
                "has_drift": s.has_drift(),
                "status": match &s.status {
                    ScanStatus::Local => serde_json::json!({"kind": "local"}),
                    ScanStatus::Installed { remote, version } => {
                        serde_json::json!({"kind": "installed", "remote": remote, "version": version})
                    }
                    ScanStatus::InstalledModified { remote, version } => {
                        serde_json::json!({"kind": "installed_modified", "remote": remote, "version": version})
                    }
                    ScanStatus::PushedLocal { remote, branch, pr_url, commit_sha } => {
                        let kind = if pr_url.is_empty() { "pushed_direct" } else { "pushed_local" };
                        serde_json::json!({
                            "kind": kind,
                            "remote": remote,
                            "branch": branch,
                            "pr_url": pr_url,
                            "commit_sha": commit_sha,
                        })
                    }
                }
            })
        })
        .collect();
    serde_json::Value::Array(arr)
}
