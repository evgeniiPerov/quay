pub mod add;
pub mod create;
pub mod info;
pub mod init;
pub mod link;
pub mod list;
pub mod outdated;
pub mod profile;
pub mod push;
pub mod remote;
pub mod remove;
pub mod search;
pub mod sync;
pub mod tui;
pub mod update;
pub mod validate;

/// Shared "no remotes configured" guard. Returns Err with an actionable hint when
/// the merged config has zero remotes. Used by commands that need at least one
/// remote to do meaningful work (add, info, search).
pub(crate) fn ensure_remotes_configured(
    cfg: &quay_core::Config,
) -> Result<(), Box<dyn std::error::Error>> {
    if cfg.remotes.is_empty() {
        return Err(
            "no remotes configured — run `quay remote add <name> <url> --default` first".into(),
        );
    }
    Ok(())
}
