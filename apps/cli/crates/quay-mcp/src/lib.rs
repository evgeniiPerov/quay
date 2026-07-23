//! MCP server for quay. Exposes skill-registry operations as MCP tools
//! over stdio. Invoked via `quay mcp`.
//!
//! IMPORTANT: stdout is the MCP protocol channel. Nothing in this crate may
//! print to stdout. All diagnostics go to stderr (eprintln!).

mod ctx;
mod install;
mod params;
mod server;

pub(crate) use ctx::ServerCtx;

use std::path::PathBuf;

pub use install::{install_client, Client};
pub use params::WriteResult;
pub use server::QuayServer;

#[doc(hidden)]
pub mod test_support {
    use crate::server::QuayServer;
    use crate::ServeOptions;
    use std::sync::atomic::{AtomicU64, Ordering};

    // This module is part of the public (non-`cfg(test)`) build so the
    // integration tests in `tests/` can reach it, therefore it may only use
    // normal dependencies — not the `tempfile` dev-dependency. We build a
    // unique path from the pid plus an atomic counter instead.
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A server rooted at a fresh throwaway temp dir with no remotes
    /// configured. Suitable for `list_tools` and schema assertions (no tool
    /// execution). Each call gets a unique directory so write-capable tools
    /// added in later tasks can't interfere across concurrent tests.
    pub fn test_server() -> QuayServer {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("quay-mcp-test-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        QuayServer::new(ServeOptions {
            project: dir,
            user_config: None,
            profile: None,
        })
    }

    /// A server rooted at the given project directory. Used by integration
    /// tests that build a real project + hub on disk and execute write tools.
    pub fn server_at(project: &std::path::Path) -> QuayServer {
        QuayServer::new(ServeOptions {
            project: project.to_path_buf(),
            user_config: None,
            profile: None,
        })
    }
}

/// Parameters captured from the CLI launch, forwarded to every tool.
#[derive(Clone)]
pub struct ServeOptions {
    pub project: PathBuf,
    pub user_config: Option<PathBuf>,
    pub profile: Option<String>,
}

/// Run the MCP server over stdio. Blocks until the client disconnects.
///
/// Builds its own Tokio runtime so the surrounding CLI stays synchronous.
pub fn serve_blocking(opts: ServeOptions) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(server::serve(opts))
}
