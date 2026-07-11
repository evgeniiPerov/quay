//! Server context: turns launch parameters into the `Config` and fetcher
//! every tool needs. Mirrors the wiring in `quay-cli`'s command modules.

use quay_core::{CloneFetcher, Config};
use std::path::{Path, PathBuf};

/// Captured once per server process; cloned cheaply into each tool call.
#[derive(Clone)]
pub struct ServerCtx {
    pub project: PathBuf,
    pub user_config: Option<PathBuf>,
    pub profile: Option<String>,
}

impl ServerCtx {
    /// Load the resolved config (user + project + profile overlay), using the
    /// server's launch profile.
    pub fn load_config(&self) -> quay_core::Result<Config> {
        self.load_config_with(None)
    }

    /// Like [`load_config`](Self::load_config) but a per-call `profile_override`
    /// (when `Some`) takes precedence over the server's launch profile — lets a
    /// single tool call target a different profile's remotes.
    pub fn load_config_with(&self, profile_override: Option<&str>) -> quay_core::Result<Config> {
        let project_config = self.project.join(".quay/config.toml");
        Config::load_resolved(
            self.user_config.as_deref(),
            Some(&project_config),
            profile_override.or(self.profile.as_deref()),
        )
    }

    /// The user-level config directory (parent of `config.toml`), used by
    /// `outdated` to locate the push-log. `None` when no user config is set.
    pub fn config_dir(&self) -> Option<&Path> {
        self.user_config.as_deref().and_then(|p| p.parent())
    }

    /// A fresh fetcher. `CloneFetcher` shallow-clones each remote on demand.
    pub fn fetcher(&self) -> CloneFetcher {
        CloneFetcher::new()
    }
}

impl From<crate::ServeOptions> for ServerCtx {
    fn from(o: crate::ServeOptions) -> Self {
        Self {
            project: o.project,
            user_config: o.user_config,
            profile: o.profile,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_config_with_no_files_present() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ServerCtx {
            project: dir.path().to_path_buf(),
            user_config: None,
            profile: None,
        };
        // No config files exist → default config, no error.
        let cfg = ctx.load_config().expect("default config loads");
        assert!(cfg.remotes.is_empty());
    }

    #[test]
    fn per_call_profile_override_beats_launch_profile() {
        // User config with two profiles, each exposing a differently-named remote.
        let dir = tempfile::tempdir().unwrap();
        let user_cfg = dir.path().join("config.toml");
        std::fs::write(
            &user_cfg,
            r#"
[profiles.alpha.remotes.ra]
url = "https://example.com/a.git"
default = true

[profiles.beta.remotes.rb]
url = "https://example.com/b.git"
default = true
"#,
        )
        .unwrap();
        let ctx = ServerCtx {
            project: dir.path().to_path_buf(),
            user_config: Some(user_cfg),
            profile: Some("alpha".into()),
        };

        // No override → launch profile (alpha) wins.
        let cfg = ctx.load_config().unwrap();
        assert!(cfg.remotes.contains_key("ra") && !cfg.remotes.contains_key("rb"));

        // Override → beta's remotes, not the launch profile's.
        let cfg = ctx.load_config_with(Some("beta")).unwrap();
        assert!(cfg.remotes.contains_key("rb") && !cfg.remotes.contains_key("ra"));
    }

    #[test]
    fn config_dir_is_parent_of_user_config() {
        let ctx = ServerCtx {
            project: "/tmp/x".into(),
            user_config: Some("/home/u/.config/quay/config.toml".into()),
            profile: None,
        };
        assert_eq!(
            ctx.config_dir(),
            Some(std::path::Path::new("/home/u/.config/quay"))
        );
    }
}
