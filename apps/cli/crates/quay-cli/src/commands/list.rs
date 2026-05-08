use quay_core::Lockfile;
use std::path::Path;

pub fn run(project: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let lock_path = project.join(".agents/skills.lock.json");
    let lock = Lockfile::load_or_default(&lock_path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&lock)?);
    } else if lock.skills.is_empty() {
        println!("(no skills installed)");
    } else {
        for (name, s) in &lock.skills {
            println!("{:<32} {:<12} (from {})", name, s.version, s.remote);
        }
    }
    Ok(())
}
