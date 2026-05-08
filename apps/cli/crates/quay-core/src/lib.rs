//! Quay domain logic.

pub mod config;
pub mod error;
pub mod fetcher;
pub mod git;
pub mod github;
pub mod lockfile;
pub mod manager;
pub mod manifest;
pub mod outdated;
pub mod provider;
pub mod pusher;
pub mod registry;
pub mod search;

pub use config::{
    Config, InstallConfig, ProfileFile, ProjectConfigFile, RemoteConfig, UserConfigFile,
    UserSection,
};
pub use error::{QuayError, Result};
pub use fetcher::{RegistryFetcher, SkillFileFetcher};
pub use git::{GitClient, GitShellClient};
pub use github::GithubRawFetcher;
#[cfg(debug_assertions)]
pub use github::GithubRawFetcherWithBase;
pub use lockfile::{LockedFile, LockedRemote, LockedSkill, Lockfile};
pub use manager::{sha256_hex, RefetchedFile, SkillManager};
pub use manifest::{parse_skill, QuayMeta, SkillManifest};
pub use outdated::{outdated, OutdatedEntry};
#[cfg(any(test, debug_assertions))]
pub use provider::FakeOpener;
pub use provider::{GhCliOpener, PrInfo, PrOpener};
pub use pusher::{BumpKind, PushResult, SkillPusher};
pub use registry::{Registry, RegistryEntry};
pub use search::{search, SearchFilters, SearchHit};
