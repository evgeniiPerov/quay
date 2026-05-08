use thiserror::Error;

#[derive(Debug, Error)]
pub enum QuayError {
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid frontmatter in {path}: {reason}")]
    InvalidFrontmatter { path: String, reason: String },
    #[error("invalid registry json: {reason}")]
    InvalidRegistry { reason: String },
    #[error("invalid config in {path}: {reason}")]
    InvalidConfig { path: String, reason: String },
    #[error("invalid lockfile: {reason}")]
    InvalidLockfile { reason: String },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("skill not found: {name} in remote {remote}")]
    SkillNotFound { name: String, remote: String },
    #[error("skill name collision: {name} found in remotes {remotes:?}")]
    NameCollision { name: String, remotes: Vec<String> },
    #[error("file integrity check failed for {path}: expected {expected}, got {actual}")]
    IntegrityFailure {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("remote already exists: {0}")]
    RemoteExists(String),
    #[error("remote not configured: {0}")]
    RemoteUnknown(String),
    #[error("config validation: {0}")]
    ConfigValidation(String),
    #[error("unknown profile: {0}")]
    ProfileUnknown(String),
    #[error("project requires profile '{0}'. Run: quay profile add {0}")]
    ProfileRequired(String),
    #[error("no profiles configured — run `quay profile add <name>` first")]
    NoProfiles,
    #[error(
        "ambiguous profile — multiple profiles exist; pass --profile=<name> or set active_profile"
    )]
    AmbiguousProfile,
}

pub type Result<T> = std::result::Result<T, QuayError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_includes_path() {
        let err = QuayError::InvalidFrontmatter {
            path: "skills/foo/SKILL.md".into(),
            reason: "missing field 'name'".into(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("skills/foo/SKILL.md"));
        assert!(msg.contains("missing field 'name'"));
    }

    #[test]
    fn from_reqwest_works() {
        // QuayError::Network has #[from] reqwest::Error.
        // Cannot construct reqwest::Error directly; just ensure the variant exists.
        fn _assert_from(_e: reqwest::Error) -> QuayError {
            _e.into()
        }
    }
}
