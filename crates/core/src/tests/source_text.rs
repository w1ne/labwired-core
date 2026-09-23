// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One home for "read Rust source as code, not as prose".
//!
//! Three gates in this directory count occurrences of a pattern across the
//! tree: [`super::event_scheduler_cfg_ratchet`], [`super::downcast_ratchet`]
//! and [`super::jit_translate_coverage_ratchet`]. All three have the same
//! failure mode — a mention of the pattern in a doc comment, an `assert!`
//! message or a test fixture counts as a site, so the gate measures the word
//! rather than the call. `downcast_ratchet` hit exactly that: a comment
//! containing the literal `as_any()` pushed the count past its ceiling while
//! the number of real call sites went down.
//!
//! The three had drifted apart before they were brought here: the copy in
//! `jit_translate_coverage_ratchet` handled no raw strings at all, so a
//! `r"..."` fixture was code to it. A gate is only as honest as its reader, and
//! a reader with two definitions has one that nobody is testing.

/// Blank every comment and string/char literal body, preserving byte offsets.
///
/// Everything downstream runs on the result, so a mention of the feature in a
/// doc comment, an `assert!` message or a test fixture cannot be a site. Raw
/// strings (`r"..."`, `r#"..."#`) and nested block comments are handled because
/// this file's own fixtures use both.
pub(crate) fn strip_comments_and_strings(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for p in from..to.min(out.len()) {
            if out[p] != b'\n' {
                out[p] = b' ';
            }
        }
    };
    while i < b.len() {
        // Raw string: r"..." or r#"..."# (any number of hashes).
        if b[i] == b'r' && i + 1 < b.len() && (b[i + 1] == b'"' || b[i + 1] == b'#') {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < b.len() && b[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < b.len() && b[j] == b'"' {
                j += 1;
                let mut term = String::from("\"");
                term.push_str(&"#".repeat(hashes));
                let end = src[j..]
                    .find(&term)
                    .map(|k| j + k + term.len())
                    .unwrap_or(b.len());
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }
        // Ordinary string literal (covers char literals closely enough: a
        // `'"'` would be a lone quote, which we terminate at the next quote —
        // and no `cfg` predicate hides inside a char literal).
        if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    break;
                }
                j += 1;
            }
            let end = (j + 1).min(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        // Line comment (`//`, `///`, `//!`).
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            let end = src[i..].find('\n').map(|k| i + k).unwrap_or(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        // Block comment, nesting as Rust does.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < b.len() && depth > 0 {
                if b[j] == b'/' && j + 1 < b.len() && b[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                } else if b[j] == b'*' && j + 1 < b.len() && b[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j.min(b.len()));
            i = j;
            continue;
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::strip_comments_and_strings;

    /// Every construct that can hide a pattern from a counting gate. The raw
    /// string case is the one the `jit_translate_coverage_ratchet` copy of this
    /// function got wrong, so it is spelled out rather than folded in.
    #[test]
    fn prose_is_blanked_but_code_survives() {
        let src = r##"
            // as_any() in a line comment
            /// as_any() in a doc comment
            /* as_any() in a block /* nested */ comment */
            let msg = "as_any() in a string";
            let raw = r"as_any() in a raw string";
            let hashed = r#"as_any() in a hashed raw string"#;
            let real = thing.as_any();
        "##;
        let stripped = strip_comments_and_strings(src);
        assert_eq!(
            stripped.matches("as_any()").count(),
            1,
            "only the call on the last line is code; stripped source was:\n{stripped}"
        );
    }

    /// Offsets must be preserved, because the cfg scan slices the stripped text
    /// and reads the original at the same index.
    #[test]
    fn blanking_preserves_length_and_lines() {
        let src = "let a = 1; // comment\nlet b = \"text\";\n";
        let stripped = strip_comments_and_strings(src);
        assert_eq!(stripped.len(), src.len(), "byte offsets shifted");
        assert_eq!(
            stripped.lines().count(),
            src.lines().count(),
            "newlines must survive blanking"
        );
        assert!(stripped.contains("let a = 1;"), "code was blanked");
    }
}
