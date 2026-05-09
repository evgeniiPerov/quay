//! Quay domain logic.

pub mod config;
pub mod error;
pub mod fetcher;
pub mod git;
pub mod github;
pub mod linker;
pub mod lockfile;
pub mod manager;
pub mod manifest;
pub mod outdated;
pub mod provider;
pub mod providers;
pub mod push_log;
pub mod pusher;
pub mod registry;
pub mod registry_builder;
pub mod scanner;
pub mod search;

pub use config::{
    Config, InstallConfig, MetaSection, MirrorConfig, MirrorStrategy, ProfileFile,
    ProjectConfigFile, PushMode, RemoteConfig, UserConfigFile, UserSection,
};
pub use error::{QuayError, Result};
pub use fetcher::{RegistryFetcher, SkillFileFetcher};
pub use git::{GitClient, GitShellClient};
pub use github::GithubRawFetcher;
#[cfg(debug_assertions)]
pub use github::GithubRawFetcherWithBase;
pub use linker::{apply_all, apply_one, check, MirrorAction, MirrorDrift};
pub use lockfile::{LockedFile, LockedRemote, LockedSkill, Lockfile};
pub use manager::{sha256_hex, RefetchedFile, SkillManager};
pub use manifest::{parse_skill, QuayMeta, SkillManifest};
pub use outdated::{outdated, OutdatedEntry};
#[cfg(any(test, debug_assertions))]
pub use provider::FakeOpener;
pub use provider::{
    detect_kind_from_url, provider_for_remote, ConnectionStatus, GhCliOpener, PrInfo, PrOpener,
    Provider, ProviderKind, RepoCoords,
};
pub use pusher::{BumpKind, PushResult, SkillPusher};
pub use registry::{Registry, RegistryEntry};
pub use search::{search, SearchFilters, SearchHit};
