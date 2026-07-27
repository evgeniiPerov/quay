//! Unified diff between two byte strings. Pure.
//!
//! Parameters are named by POSITION (`old`, `new`), never by side. Callers
//! choose which side is which: `reconcile` is push-oriented (old = harbor, so
//! `+` is what you would send) while `reconcile::folder` is pull-oriented
//! (old = local, so `+` is what the hub would give you). Side-named fields
//! would silently hold the wrong value for one of them.

use similar::{ChangeTag, TextDiff};

/// Rendered diff, or a note when content is not UTF-8 text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// Unified text diff (may be empty string when identical).
    Text(String),
    /// Non-UTF8 / binary: human note instead of a body. Sizes follow the
    /// argument positions, so the caller labels them.
    Binary { old_bytes: usize, new_bytes: usize },
}

pub fn render(old: &[u8], new: &[u8]) -> Diff {
    let (Ok(h), Ok(l)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return Diff::Binary {
            old_bytes: old.len(),
            new_bytes: new.len(),
        };
    };
    let td = TextDiff::from_lines(h, l);
    let mut out = String::new();
    for change in td.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.value());
        if !change.value().ends_with('\n') {
            out.push('\n');
        }
    }
    Diff::Text(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_when_non_utf8() {
        let d = render(&[0xff, 0xfe], b"text");
        assert_eq!(
            d,
            Diff::Binary {
                old_bytes: 2,
                new_bytes: 4
            }
        );
    }

    #[test]
    fn identical_text_has_no_signs() {
        let Diff::Text(s) = render(b"a\nb\n", b"a\nb\n") else {
            panic!()
        };
        assert!(!s.contains("\n-"));
        assert!(!s.contains("\n+"));
    }

    #[test]
    fn changed_text_is_rendered() {
        let Diff::Text(s) = render(b"line1\nline2\n", b"line1\nCHANGED\n") else {
            panic!()
        };
        // deterministic inline assertions (NO external snapshot file — avoids
        // cargo-insta tooling + untracked .snap files in batch mode):
        assert!(
            s.contains("-line2\n"),
            "expected deletion of line2, got:\n{s}"
        );
        assert!(
            s.contains("+CHANGED\n"),
            "expected insertion of CHANGED, got:\n{s}"
        );
        assert!(
            s.contains(" line1\n"),
            "expected unchanged line1 context, got:\n{s}"
        );
    }
}
