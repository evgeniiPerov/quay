//! Implementation of `quay tui`.

use quay_core::Config;
use std::path::Path;

pub fn run(
    project: &Path,
    user_config: Option<&Path>,
    profile: Option<&str>,
    check_config_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if check_config_only {
        let needs_onboarding = match crate::config_io::read_user_file(user_config) {
            Ok(file) => crate::tui::app::should_show_onboarding(&file),
            Err(_) => true,
        };
        std::process::exit(if needs_onboarding { 2 } else { 0 });
    }

    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    let app = crate::tui::app::App::new(
        cfg,
        project.to_path_buf(),
        user_config.map(|p| p.to_path_buf()),
    );
    crate::tui::run(app)?;
    Ok(())
}
