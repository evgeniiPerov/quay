use crate::error::Result;
use crate::registry::Registry;

/// Fetches the registry.json catalog from a hub.
pub trait RegistryFetcher {
    /// Fetch the registry at the hub's default ref (typically the main branch).
    fn fetch(&self, hub_url: &str) -> Result<Registry>;

    /// Fetch the registry pinned to a specific git ref (branch, tag, or SHA).
    /// Default implementation falls through to [`Self::fetch`]; concrete
    /// fetchers should override this when they can target a specific ref.
    fn fetch_at(&self, hub_url: &str, _git_ref: &str) -> Result<Registry> {
        self.fetch(hub_url)
    }
}

/// Fetches a single file (e.g., SKILL.md) from a hub at a given path.
pub trait SkillFileFetcher {
    /// Fetch the file at the hub's default ref (typically the main branch).
    fn fetch_file(&self, hub_url: &str, path: &str) -> Result<Vec<u8>>;

    /// Fetch the file pinned to a specific git ref (commit SHA, tag, or branch).
    /// Default implementation falls through to [`Self::fetch_file`]; concrete
    /// fetchers should override this when they can target a specific ref.
    fn fetch_file_at(&self, hub_url: &str, path: &str, _git_ref: &str) -> Result<Vec<u8>> {
        self.fetch_file(hub_url, path)
    }
}
