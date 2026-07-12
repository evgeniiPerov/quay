//! Quay domain logic.

pub mod add_plan;
pub mod agents;
pub mod clone_fetcher;
pub mod config;
pub mod error;
pub mod fetcher;
pub mod git;
pub mod github;
pub mod linker;
pub mod lock;
pub mod lock_hash;
pub mod manager;
pub mod manifest;
pub mod outdated;
pub mod profile_draft;
pub mod provider;
pub mod providers;
pub mod push_log;
pub mod pusher;
pub mod reconcile;
pub mod registry;
pub mod registry_builder;
pub mod scanner;
pub mod search;
pub mod skill_files;
pub mod validate;

pub use add_plan::{
    build_plan, build_plan_with_prompt, collision_names, CollisionStrategy, SkillAction,
};
pub use agents::{
    detect_installed, install_config, registry as agent_registry, Agent, Registry as AgentRegistry,
    Scope as AgentScope,
};
pub use clone_fetcher::CloneFetcher;
pub use config::{
    Config, InstallConfig, MetaSection, MirrorConfig, MirrorRoot, MirrorStrategy, ProfileFile,
    ProjectConfigFile, PushMode, RemoteConfig, UserConfigFile, UserSection,
};
pub use error::{QuayError, Result};
pub use fetcher::{RegistryFetcher, SkillFileFetcher};
pub use git::{GitClient, GitShellClient};
/// Legacy HTTP fetcher for GitHub raw URLs. Kept for backward compatibility;
/// prefer [`CloneFetcher`] for new code.
#[deprecated(since = "0.2.0", note = "use CloneFetcher instead")]
pub use github::GithubRawFetcher;
#[cfg(debug_assertions)]
pub use github::GithubRawFetcherWithBase;
pub use linker::{
    apply_all, apply_one, check, classify, reconcile, MirrorAction, MirrorDrift, MirrorState,
    ReconcileReport,
};
pub use lock::{
    read as read_lock, source_from_url, write_atomic as write_lock, LockEntry, SkillsLock,
    SourceType,
};
pub use lock_hash::folder_hash;
pub use manager::{sha256_hex, SkillManager};
pub use manifest::{parse_skill, QuayMeta, SkillManifest};
pub use outdated::{outdated_for_local, OutdatedEntry};
pub use profile_draft::{ProfileDraft, RemoteDraft};
#[cfg(any(test, debug_assertions))]
pub use provider::FakeOpener;
pub use provider::{
    detect_kind_from_url, provider_for_remote, ConnectionStatus, GhCliOpener, PrInfo, PrOpener,
    Provider, ProviderKind, RepoCoords,
};
pub use pusher::{BumpKind, PushResult, SkillPusher};
pub use registry::{Registry, RegistryEntry};
pub use search::{search, SearchFilters, SearchHit};
