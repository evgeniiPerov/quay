//! `quay scan` — list local skills and their sync status.

use quay_core::lockfile::Lockfile;
use quay_core::push_log::PushLog;
use quay_core::scanner::{scan_local_skills, LocalSkill, ScanStatus, SkillFormat};
use std::path::{Path, PathBuf};

pub fn run(
    project_root: &Path,
    root: Option<PathBuf>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let scan_root = root.unwrap_or_else(|| project_root.join(".agents/skills"));

    // Best-effort loads: any read error => empty defaults.
    let lockfile =
        Lockfile::load_or_default(&project_root.join(".quay/lockfile.json")).unwrap_or_default();
    let push_log = PushLog::load(project_root).unwrap_or_default();

    let skills = scan_local_skills(&[scan_root], &lockfile, &push_log);

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
        println!("(no local skills found under .agents/skills/)");
        return;
    }
    println!("{:<32}  {:<14}  {:<28}  PATH", "NAME", "FORMAT", "STATUS");
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
                pr_url,
                commit_sha,
                ..
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
        println!(
            "{:<32}  {:<14}  {:<28}  {}",
            s.meta.name,
            format,
            status,
            s.path.display()
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
                "path": s.path.display().to_string(),
                "sha256": s.sha256,
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
