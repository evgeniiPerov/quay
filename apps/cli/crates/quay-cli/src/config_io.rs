//! Shared read/write helpers for the user and project config files. Used by
//! both the CLI subcommands (`profile`, `link`, `remote`) and the TUI Settings
//! screen so writes always go through the same atomic-rename code path.

use quay_core::{ProjectConfigFile, QuayError, UserConfigFile};
use std::path::Path;

/// Read the user config (legacy or new shape) and migrate legacy fields in place.
/// Returns an empty `UserConfigFile` if the path is `None` or the file does not exist.
pub fn read_user_file(path: Option<&Path>) -> Result<UserConfigFile, QuayError> {
    let p = match path {
        Some(p) => p,
        None => return Ok(UserConfigFile::default()),
    };
    if !p.exists() {
        return Ok(UserConfigFile::default());
    }
    let text = std::fs::read_to_string(p).map_err(|source| QuayError::Io {
        path: p.display().to_string(),
        source,
    })?;
    let mut file: UserConfigFile = toml::from_str(&text).map_err(|e| QuayError::InvalidConfig {
        path: p.display().to_string(),
        reason: e.to_string(),
    })?;
    file.migrate_legacy_in_place();
    Ok(file)
}

/// Atomically write the user config file. Creates parent directories as needed.
pub fn write_user_file(path: &Path, file: &UserConfigFile) -> Result<(), QuayError> {
    let text = toml::to_string_pretty(file).map_err(|e| QuayError::InvalidConfig {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    write_atomic(path, &text)
}

/// Read the project config file. Returns an empty `ProjectConfigFile` if missing.
pub fn read_project_file(project: &Path) -> Result<ProjectConfigFile, QuayError> {
    let path = project.join(".quay/config.toml");
    if !path.exists() {
        return Ok(ProjectConfigFile::default());
    }
    let text = std::fs::read_to_string(&path).map_err(|source| QuayError::Io {
        path: path.display().to_string(),
        source,
    })?;
    toml::from_str(&text).map_err(|e| QuayError::InvalidConfig {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

/// Atomically write the project config file.
pub fn write_project_file(project: &Path, file: &ProjectConfigFile) -> Result<(), QuayError> {
    let path = project.join(".quay/config.toml");
    let text = toml::to_string_pretty(file).map_err(|e| QuayError::InvalidConfig {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    write_atomic(&path, &text)
}

fn write_atomic(path: &Path, text: &str) -> Result<(), QuayError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|source| QuayError::Io {
        path: tmp.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| QuayError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;

    #[test]
    fn round_trip_user_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let p = dir.child("user.toml");
        let file = UserConfigFile {
            active_profile: Some("work".into()),
            ..Default::default()
        };
        write_user_file(p.path(), &file).unwrap();
        let read = read_user_file(Some(p.path())).unwrap();
        assert_eq!(read.active_profile.as_deref(), Some("work"));
    }

    #[test]
    fn read_user_file_missing_path_returns_default() {
        let r = read_user_file(None).unwrap();
        assert!(r.profiles.is_empty());
    }

    #[test]
    fn read_user_file_nonexistent_path_returns_default() {
        let dir = assert_fs::TempDir::new().unwrap();
        let p = dir.child("nope.toml");
        let r = read_user_file(Some(p.path())).unwrap();
        assert!(r.profiles.is_empty());
    }

    #[test]
    fn round_trip_project_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let file = ProjectConfigFile {
            profile: Some("work".into()),
            ..Default::default()
        };
        write_project_file(dir.path(), &file).unwrap();
        let read = read_project_file(dir.path()).unwrap();
        assert_eq!(read.profile.as_deref(), Some("work"));
    }
}
