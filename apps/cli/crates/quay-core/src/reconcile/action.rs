//! What the user chose for a single colliding skill, and how to apply it.

use crate::error::{QuayError, Result};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveAction {
    /// Overwrite the local file with harbor-HEAD bytes.
    Replace,
    /// Keep the local file (deliberate). No write.
    Keep,
    /// Undecided. No write.
    Skip,
}

impl ResolveAction {
    /// Maps to the existing add_plan action so the batch pipeline is unchanged:
    /// Replace -> overwrite (force), Keep/Skip -> leave local.
    pub fn writes(self) -> bool {
        matches!(self, ResolveAction::Replace)
    }
}

/// Apply `Replace` by writing `head_bytes` to `local_path`. No-op otherwise.
pub fn apply(action: ResolveAction, local_path: &Path, head_bytes: &[u8]) -> Result<()> {
    if action.writes() {
        std::fs::write(local_path, head_bytes).map_err(|source| QuayError::Io {
            path: local_path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn replace_writes_hub_bytes() {
        let d = tempdir().unwrap();
        let p = d.path().join("SKILL.md");
        std::fs::write(&p, b"local").unwrap();
        apply(ResolveAction::Replace, &p, b"hub").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hub");
    }

    #[test]
    fn keep_and_skip_do_not_write() {
        let d = tempdir().unwrap();
        let p = d.path().join("SKILL.md");
        std::fs::write(&p, b"local").unwrap();
        apply(ResolveAction::Keep, &p, b"hub").unwrap();
        apply(ResolveAction::Skip, &p, b"hub").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"local");
    }
}
