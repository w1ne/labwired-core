// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `uart_regex` replaced a `^ $ . *`-only matcher with a real engine. Every
//! assertion anyone has already written must keep meaning exactly what it meant.
//!
//! This test carries a verbatim copy of the **old** implementation and uses it
//! as an oracle: for every pattern in the old engine's language, across a
//! corpus of texts, old and new must agree. The oracle is the previous
//! behaviour itself, not a restatement of the new code's logic — a mirror test
//! would prove nothing here.
//!
//! ## The scope of the equivalence claim
//!
//! It covers the old engine's actual language: literals, `.`, `*` after an
//! atom, a leading `^`, and a trailing `$`.
//!
//! It deliberately does **not** cover patterns the old engine only ever matched
//! by accident, because it had no syntax for them:
//!
//! - `+ ? [ ] ( ) | \` were literal characters. They are now metacharacters.
//! - `*` with nothing before it was a literal `*`. It is now a parse error.
//! - `$` anywhere but the end, and `^` anywhere but the start, were literals.
//!   They are now anchors.
//!
//! Those are the intended change. [`every_uart_regex_in_the_repo_is_unaffected`]
//! is what makes that change safe in practice: it asserts every pattern
//! actually checked into this repository lives in the equivalent subset.

use labwired_cli::regex;

/// The pre-change matcher, copied verbatim from `simple_regex_is_match`.
/// Retained ONLY as the differential oracle for the tests below.
fn legacy_is_match(pattern: &str, text: &str) -> bool {
    fn char_eq(pat: char, ch: char) -> bool {
        pat == '.' || pat == ch
    }

    fn match_here(pat: &[char], text: &[char]) -> bool {
        if pat.is_empty() {
            return true;
        }
        if pat.len() >= 2 && pat[1] == '*' {
            return match_star(pat[0], &pat[2..], text);
        }
        if pat[0] == '$' && pat.len() == 1 {
            return text.is_empty();
        }
        if !text.is_empty() && char_eq(pat[0], text[0]) {
            return match_here(&pat[1..], &text[1..]);
        }
        false
    }

    fn match_star(ch: char, pat: &[char], text: &[char]) -> bool {
        let mut i = 0;
        loop {
            if match_here(pat, &text[i..]) {
                return true;
            }
            if i >= text.len() {
                return false;
            }
            if !char_eq(ch, text[i]) {
                return false;
            }
            i += 1;
        }
    }

    let pat_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    if pat_chars.first().copied() == Some('^') {
        return match_here(&pat_chars[1..], &text_chars);
    }

    for start in 0..=text_chars.len() {
        if match_here(&pat_chars, &text_chars[start..]) {
            return true;
        }
    }
    false
}

/// Every pattern in the old engine's language, up to `max_atoms` atoms: each
/// atom is `a`, `b` or `.`, optionally followed by `*`, with an optional
/// leading `^` and trailing `$`.
fn legacy_language(max_atoms: usize) -> Vec<String> {
    let mut bodies = vec![String::new()];
    for _ in 0..max_atoms {
        let mut next = Vec::new();
        for body in &bodies {
            for atom in ['a', 'b', '.'] {
                for star in ["", "*"] {
                    next.push(format!("{body}{atom}{star}"));
                }
            }
        }
        bodies.extend(next);
    }

    let mut out = Vec::new();
    for body in bodies {
        for prefix in ["", "^"] {
            for suffix in ["", "$"] {
                out.push(format!("{prefix}{body}{suffix}"));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Texts over `a`/`b` up to `max_len`, plus a few with foreign characters so
/// `.` and literal mismatches are exercised too.
fn corpus_texts(max_len: usize) -> Vec<String> {
    let mut texts = vec![String::new()];
    let mut frontier = vec![String::new()];
    for _ in 0..max_len {
        let mut next = Vec::new();
        for t in &frontier {
            for c in ['a', 'b'] {
                next.push(format!("{t}{c}"));
            }
        }
        texts.extend(next.clone());
        frontier = next;
    }
    texts.extend(["x".into(), "ax".into(), "xa".into(), "abxba".into()]);
    texts
}

#[test]
fn the_new_engine_is_identical_on_the_old_engines_whole_language() {
    let patterns = legacy_language(4);
    let texts = corpus_texts(5);
    assert!(
        patterns.len() > 1000 && texts.len() > 60,
        "the differential corpus collapsed — it would pass vacuously \
         ({} patterns, {} texts)",
        patterns.len(),
        texts.len()
    );

    let mut compared = 0u64;
    for pattern in &patterns {
        for text in &texts {
            let old = legacy_is_match(pattern, text);
            let new = regex::is_match(pattern, text).unwrap_or_else(|e| {
                panic!(
                    "pattern {pattern:?} is in the old language but the new engine rejects it: {e}"
                )
            });
            assert_eq!(
                old, new,
                "DIVERGENCE: pattern {pattern:?} against {text:?} — old={old}, new={new}"
            );
            compared += 1;
        }
    }
    assert!(compared > 60_000, "only {compared} comparisons ran");
}

#[test]
fn every_uart_regex_in_the_repo_is_unaffected() {
    // The patterns actually checked in, as of this change. If someone adds one
    // that uses new syntax, it belongs in the regex module's own tests — this
    // list is specifically "patterns written against the OLD engine".
    let existing = [
        ".*",
        "MOOD=ACTIVE",
        "MOOD=CALM",
        "^Hello.*$",
        "^ThisTextWillNeverBeFound$",
    ];
    let texts = [
        "",
        "Hello world",
        "MOOD=ACTIVE",
        "MOOD=CALM",
        "mood=active",
        "Hello",
        "prefix MOOD=ACTIVE suffix",
        "ThisTextWillNeverBeFound",
    ];

    for pattern in existing {
        // None of them use syntax whose meaning changed.
        assert!(
            !pattern.contains(['+', '?', '[', ']', '(', ')', '|', '\\', '{']),
            "checked-in pattern {pattern:?} uses a character that is now a \
             metacharacter — its meaning may have changed"
        );
        for text in texts {
            assert_eq!(
                legacy_is_match(pattern, text),
                regex::is_match(pattern, text).expect("checked-in pattern must compile"),
                "DIVERGENCE on a checked-in pattern {pattern:?} against {text:?}"
            );
        }
    }
}

#[test]
fn the_oracle_is_not_vacuous() {
    // If `legacy_is_match` were accidentally reduced to something trivial, the
    // differential above would pass while proving nothing. Pin a few known
    // old-engine answers.
    assert!(legacy_is_match("^Hello.*$", "Hello world"));
    assert!(!legacy_is_match("^Hello.*$", "Goodbye"));
    assert!(legacy_is_match("ab*c", "ac"));
    assert!(!legacy_is_match("MOOD=CALM", "MOOD=ACTIVE"));
    // …and that it really is the OLD engine: it treats `+` as a literal.
    assert!(legacy_is_match("a+b", "a+b"));
    assert!(!legacy_is_match("a+b", "aab"));
    // The new engine, by contrast, reads it as a quantifier. This asserts the
    // upgrade actually happened.
    assert!(regex::is_match("a+b", "aab").unwrap());
}
