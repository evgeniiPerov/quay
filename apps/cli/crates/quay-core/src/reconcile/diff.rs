//! Unified diff: harbor-HEAD bytes (old) vs local bytes (new). Pure.

use similar::{ChangeTag, TextDiff};

/// Rendered diff, or a note when content is not UTF-8 text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// Unified text diff (may be empty string when identical).
    Text(String),
    /// Non-UTF8 / binary: human note instead of a body.
    Binary {
        hub_bytes: usize,
        local_bytes: usize,
    },
}

pub fn render(hub: &[u8], local: &[u8]) -> Diff {
    let (Ok(h), Ok(l)) = (std::str::from_utf8(hub), std::str::from_utf8(local)) else {
        return Diff::Binary {
            hub_bytes: hub.len(),
            local_bytes: local.len(),
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
                hub_bytes: 2,
                local_bytes: 4
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
