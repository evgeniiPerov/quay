//! Unified diff between two byte strings. Pure.
//!
//! Parameters are named by POSITION (`old`, `new`), never by side. Callers
//! choose which side is which: `reconcile` is push-oriented (old = harbor, so
//! `+` is what you would send) while `reconcile::folder` is pull-oriented
//! (old = local, so `+` is what the hub would give you). Side-named fields
//! would silently hold the wrong value for one of them.

use similar::{ChangeTag, TextDiff};

/// Unchanged lines kept on each side of a change, matching `git diff`'s default.
const CONTEXT_LINES: usize = 3;

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
    // Hunks with limited context, not the whole file. A skill's REFERENCE.md
    // runs to hundreds of lines, and echoing all of them buries the edit.
    for (i, hunk) in td
        .unified_diff()
        .context_radius(CONTEXT_LINES)
        .iter_hunks()
        .enumerate()
    {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&hunk.header().to_string());
        out.push('\n');
        for change in hunk.iter_changes() {
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
    fn only_nearby_context_is_kept() {
        // A skill's REFERENCE.md runs to hundreds of lines. Emitting every
        // unchanged line as context buries a one-line edit and makes the whole
        // report unreadable.
        let mut old = String::new();
        for i in 0..200 {
            old.push_str(&format!("line {i}\n"));
        }
        let new = old.replace("line 100\n", "line 100 CHANGED\n");

        let Diff::Text(s) = render(old.as_bytes(), new.as_bytes()) else {
            panic!()
        };

        assert!(s.contains("-line 100\n"), "the change itself: {s}");
        assert!(s.contains("+line 100 CHANGED\n"));
        assert!(s.contains(" line 99\n"), "nearby context is kept");
        assert!(
            !s.contains(" line 0\n"),
            "a line 100 away from the edit is not context:\n{s}"
        );
        assert!(
            s.lines().count() < 20,
            "one edit in a 200-line file should not print 200 lines, got {}",
            s.lines().count()
        );
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
