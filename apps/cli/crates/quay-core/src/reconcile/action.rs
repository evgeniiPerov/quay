//! What the user chose for a single colliding skill, and how to apply it.

use crate::error::{QuayError, Result};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveAction {
    /// Overwrite the local file with the carried harbor-HEAD bytes.
    ///
    /// The bytes ride on the variant rather than travelling to [`apply`] as a
    /// second argument. Passing them separately made `Replace` constructible
    /// without them, and the report they came from carries no bytes at all when
    /// the skill is absent on harbor HEAD — so the two could disagree and
    /// truncate the user's file to zero. Here that state cannot be written down.
    Replace(Vec<u8>),
    /// Keep the local file (deliberate). No write.
    Keep,
    /// Undecided. No write.
    Skip,
}

impl ResolveAction {
    /// Maps to the existing add_plan action so the batch pipeline is unchanged:
    /// Replace -> overwrite (force), Keep/Skip -> leave local.
    pub fn writes(&self) -> bool {
        matches!(self, ResolveAction::Replace(_))
    }
}

/// Apply `Replace` by writing its harbor-HEAD bytes to `local_path`. No-op
/// otherwise.
pub fn apply(action: &ResolveAction, local_path: &Path) -> Result<()> {
    if let ResolveAction::Replace(head_bytes) = action {
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
        apply(&ResolveAction::Replace(b"hub".to_vec()), &p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hub");
    }

    #[test]
    fn keep_and_skip_do_not_write() {
        let d = tempdir().unwrap();
        let p = d.path().join("SKILL.md");
        std::fs::write(&p, b"local").unwrap();
        apply(&ResolveAction::Keep, &p).unwrap();
        apply(&ResolveAction::Skip, &p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"local");
    }
}
