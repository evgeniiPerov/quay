//! Domain-level validators for user-supplied identifiers.
//!
//! Single source of truth for profile-name and email rules. Every entry
//! point that accepts these values from the user — interactive wizard,
//! TOML ingestion, explicit CLI flags — must call into this module so
//! all paths reject the same garbage.

use crate::error::{QuayError, Result};

/// Validate a profile name against `^[a-z0-9][a-z0-9_-]*$`, max 64 chars.
///
/// Returns [`QuayError::ConfigValidation`] with a user-readable reason on
/// failure.
pub fn profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(QuayError::ConfigValidation(
            "profile name must not be empty".into(),
        ));
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(QuayError::ConfigValidation(format!(
            "profile name '{}' must start with a lowercase letter or digit",
            name
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(QuayError::ConfigValidation(format!(
            "profile name '{}' must only contain lowercase letters, digits, hyphens, or underscores",
            name
        )));
    }
    if name.len() > 64 {
        return Err(QuayError::ConfigValidation(format!(
            "profile name exceeds 64 characters (got {})",
            name.len()
        )));
    }
    Ok(())
}

/// Loose email validation: non-empty, contains `@`, no whitespace.
///
/// Intentionally permissive — Git itself accepts almost anything in
/// `user.email`, so we reject only the clearly-broken cases.
pub fn email_loose(email: &str) -> Result<()> {
    if email.is_empty() {
        return Err(QuayError::ConfigValidation(
            "email must not be empty".into(),
        ));
    }
    if !email.contains('@') {
        return Err(QuayError::ConfigValidation("email must contain '@'".into()));
    }
    if email.chars().any(|c| c.is_whitespace()) {
        return Err(QuayError::ConfigValidation(
            "email must not contain whitespace".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_rejects_uppercase() {
        assert!(profile_name("Work").is_err());
        assert!(profile_name("WORK").is_err());
        assert!(profile_name("workSpace").is_err());
    }

    #[test]
    fn profile_name_rejects_leading_special() {
        assert!(profile_name("-work").is_err());
        assert!(profile_name("_work").is_err());
        assert!(profile_name("").is_err());
    }

    #[test]
    fn profile_name_accepts_valid() {
        assert!(profile_name("work").is_ok());
        assert!(profile_name("my-profile").is_ok());
        assert!(profile_name("work_2024").is_ok());
        assert!(profile_name("p").is_ok());
        assert!(profile_name("a1").is_ok());
    }

    #[test]
    fn profile_name_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(profile_name(&long).is_err());
    }

    #[test]
    fn email_loose_requires_at_sign() {
        assert!(email_loose("notanemail").is_err());
        assert!(email_loose("").is_err());
        assert!(email_loose("a @b.com").is_err());
        assert!(email_loose("a@b.com").is_ok());
        assert!(email_loose("x@y").is_ok());
    }

    #[test]
    fn returned_error_is_config_validation_variant() {
        match profile_name("BAD") {
            Err(QuayError::ConfigValidation(_)) => {}
            other => panic!("expected ConfigValidation, got {other:?}"),
        }
    }
}
