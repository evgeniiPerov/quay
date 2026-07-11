//! `quay list` — list locally-discovered skills.
//!
//! Plan 10: reads `scan_local` output; shows all skills (status is derived
//! from the push log, not a lockfile).

use quay_core::push_log::PushLog;
use quay_core::scanner::scan_local;
use std::path::Path;

pub fn run(project: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = crate::config_io::default_config_dir();
    let push_log =
        PushLog::load(config_dir.as_deref().unwrap_or(project), Some(project)).unwrap_or_default();
    let skills = scan_local(project, &push_log);

    if json {
        let arr: Vec<_> = skills
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.meta.name,
                    "description": s.meta.description,
                    "version": s.meta.version_display(),
                    "tags": s.meta.tags,
                    "mirrors": s.locations.iter().map(|l| l.root.label()).collect::<Vec<_>>(),
                    "canonical_path": s.canonical_path().display().to_string(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else if skills.is_empty() {
        println!("(no local skills found)");
    } else {
        for s in &skills {
            let mirrors: Vec<&str> = s.locations.iter().map(|l| l.root.label()).collect();
            println!(
                "{:<32} {:<10}  mirrors: {}",
                s.meta.name,
                s.meta.version_display(),
                mirrors.join(",")
            );
        }
    }
    Ok(())
}
