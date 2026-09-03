// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The `event-scheduler` conditional-compilation surface may not grow.
//!
//! `crates/core/Cargo.toml` declares the feature with its own end condition:
//! *"Off through 2B.N-1; flipped unconditional after every peripheral migrates
//! and the legacy walk is deleted."* That is a promise with no clock on it, and
//! the surface has only ever gone one way. Re-derived from this repo's history
//! (`git rev-list -1 --before=<date> origin/main`, counted with the same
//! comment-stripping counter this file uses):
//!
//! ```text
//!   date        sha         cfg attrs   cfg!()   TOTAL   files
//!   2026-05-01  0348a53cc           0        0       0       0
//!   2026-05-30  aa016d8ec           0        0       0       0   feature lands (#143)
//!   2026-06-01  0efb362f0          19        0      19       2
//!   2026-07-01  ff0300d8a          19        0      19       4
//!   2026-07-15  fbc155386          78       19      97      26
//!   2026-08-01  013a4c4d4         100       58     158      65
//!   2026-08-15  22574f888         134       62     196      72
//!   2026-09-01  d7b9cb010         140       68     208      77
//! ```
//!
//! Flat for six weeks, then 19 → 208 in eight. Every peripheral author now has
//! to be correct in two worlds and a given build only ever compiles one of
//! them; the wire gates already run under three feature sets to see both paths,
//! and the `e-reader` walk regression was `legacy_walk_disabled` flipping.
//!
//! The first fall is Phase 1 of the migration: 30 peripherals carried a
//! byte-identical private `fn scheduler_mode(&self) -> bool { cfg!(feature =
//! "event-scheduler") && self.clock.is_some() }`, and those 30 `cfg!` sites are
//! now the one inside [`crate::cycle_clock::scheduler_mode`] — 208 → 179, with
//! no behaviour change and no attribute touched.
//!
//! This gate does NOT decide the feature's fate, and deliberately so — that
//! decision needs a walk-differential measurement nobody has. It is the move
//! that is right under both outcomes: if the migration finishes the count falls
//! to zero and this file is deleted with it; if the feature becomes permanent
//! the surface still stops spreading unwatched.
//!
//! # What is counted, and why
//!
//! A **site** is one place the compiler is asked to pick a world:
//!
//! * `#[cfg(...)]` / `#![cfg(...)]` / `#[cfg_attr(...)]` whose predicate names
//!   the feature — the item exists in one world and not the other.
//! * `cfg!(...)` whose predicate names the feature — both arms compile, but
//!   only one is ever live, and the dead one is never type-checked *against
//!   reality*. 39 of the 179 sites are this form. Counting attributes alone
//!   would let the whole surface keep growing through `cfg!` without ever
//!   moving the number, which is a gate that governs nothing.
//!
//! `not(feature = "event-scheduler")` **counts**. It is not a third world, it
//! is the second one written from the other side, and it costs an author
//! exactly the same double-correctness. 9 of the 140 attributes are negated.
//!
//! Two ceilings, not one, because the two surfaces move for unrelated reasons
//! and a single number would let one hide the other — and because a single
//! number scoped to `crates/core/src` could be reduced by *moving* a gated
//! block into a test file rather than deleting it:
//!
//! * [`MAX_MODEL_SITES`] governs `crates/core/src/**` — the engine itself.
//! * [`MAX_HARNESS_SITES`] governs the rest of `crates/**` — chiefly
//!   `crates/core/tests/**`, plus 5 sites that have already spread into
//!   `crates/cli`.
//!
//! # What it does NOT do
//!
//! It does not say any individual site is wrong. Some are load-bearing (the
//! per-chip walk-differential gates exist *because* the two paths are not known
//! to agree). It does not migrate anything, flip the feature, or touch
//! `legacy_walk_disabled`. A count cannot tell a justified gate from debt, and
//! pretending otherwise would make this an excuse rather than a floor.
//!
//! # Why a rise fails but a fall does not
//!
//! [`crate::tests::downcast_ratchet`] additionally asserts *equality*, so a
//! shrink cannot be banked as headroom. That is right for a number expected to
//! hover. This one is expected to be driven to **zero**, in large steps, by
//! migration PRs that have no other reason to touch this file — so here a fall
//! is the goal and is allowed to land unattended. Only a rise is a fact
//! somebody has to defend, in the commit that causes it.
//!
//! # The counter ignores prose
//!
//! The same scan with [`strip_comments_and_strings`] disabled — the "naive
//! grep" figure — counts 190 model sites where the truth is 179: eleven of the
//! matches are doc comments *about* the feature (`scheduler_lane_coverage.rs`
//! quotes `#![cfg(feature = "event-scheduler")]` five times to explain the
//! vacuous-target hazard) and assertion strings. Miscounts of
//! exactly that kind have reached this repo's ledger before, so
//! [`count_in_source`] blanks comments and string literals *before* it looks
//! for anything, and [`prose_about_the_feature_is_not_a_site`] pins that
//! behaviour to fixtures. The same property means this file needs no
//! self-exclusion: its own doc comment above is invisible to it, which
//! [`this_file_contributes_no_sites`] proves rather than assumes.

use std::path::{Path, PathBuf};

/// The literal the predicates are matched against. Not `env!` — the point is
/// to notice when a *new* place starts naming this feature, including a place
/// that spells the name by hand.
const FEATURE: &str = "event-scheduler";

/// `crates/core/src/**` — the engine's own conditional-compilation surface.
///
/// Was 208 on `c8172b917` (2026-09-03) and still 208 at this branch's base:
/// 140 `cfg`/`cfg_attr` attributes (9 of them negated) plus 68 `cfg!`
/// expressions, across 77 files.
///
/// Now 179: the same 140 attributes (9 still negated) plus 39 `cfg!`, across 64
/// files. Phase 1 of the migration folded 30 byte-identical private
/// `scheduler_mode()` bodies into [`crate::cycle_clock::scheduler_mode`], so
/// 30 `cfg!` sites became the one inside that macro — a net −29 with no
/// behaviour change and no attribute touched.
///
/// LOWER this when peripherals migrate and gated blocks are deleted. RAISING it
/// requires the commit to say which of the two futures the new site serves: a
/// step in the migration that ends the feature, or another permanent fork the
/// migration will have to unpick later.
const MAX_MODEL_SITES: usize = 179;

/// The rest of `crates/**` — test harnesses and downstream crates.
///
/// Measured at 75 on `c8172b917`: 70 in `crates/core/tests/**` (67 attributes,
/// 3 `cfg!`) and 5 in `crates/cli` (2 `cfg!` in `commands/test.rs` and
/// `lib.rs`, 3 `#[cfg]` attributes in `crates/cli/tests/`).
///
/// Split from [`MAX_MODEL_SITES`] on purpose. `crates/core/tests/**` holds
/// files whose *whole body* sits behind `#![cfg(feature = "event-scheduler")]`
/// — the vacuous-target hazard `no_vacuous_test_targets` and
/// `scheduler_lane_coverage` already govern — so those move for lane reasons,
/// not model reasons. Counting them together would let a lane change mask an
/// engine regression, or vice versa.
const MAX_HARNESS_SITES: usize = 75;

// ---------------------------------------------------------------------------
// The counter. A pure function over source text, so its definition is testable
// against fixtures rather than only against the tree it happens to be run on.
// ---------------------------------------------------------------------------

/// Blank every comment and string/char literal body, preserving byte offsets.
///
/// Everything downstream runs on the result, so a mention of the feature in a
/// doc comment, an `assert!` message or a test fixture cannot be a site. Raw
/// strings (`r"..."`, `r#"..."#`) and nested block comments are handled because
/// this file's own fixtures use both.
fn strip_comments_and_strings(src: &str) -> String {
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

/// Byte index of the delimiter that closes the group opened at `open`.
fn matching_close(b: &[u8], open: usize, opens: &[u8], closes: &[u8]) -> usize {
    let mut depth = 0usize;
    let mut j = open;
    while j < b.len() {
        if opens.contains(&b[j]) {
            depth += 1;
        } else if closes.contains(&b[j]) {
            depth -= 1;
            if depth == 0 {
                return j;
            }
        }
        j += 1;
    }
    b.len().saturating_sub(1)
}

/// Does `haystack` contain `needle`? Byte-wise on purpose — see
/// [`count_in_source`] on why nothing here may slice a `&str`.
fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The identifier immediately after an attribute's `[`, with whitespace
/// skipped, up to (and including) its opening `(`. `None` if the next
/// non-whitespace run is not an `ident(`.
fn attr_head(sb: &[u8], open_bracket: usize) -> Option<&[u8]> {
    let mut j = open_bracket + 1;
    while j < sb.len() && sb[j].is_ascii_whitespace() {
        j += 1;
    }
    let start = j;
    while j < sb.len() && (sb[j].is_ascii_alphanumeric() || sb[j] == b'_') {
        j += 1;
    }
    let name = &sb[start..j];
    while j < sb.len() && sb[j].is_ascii_whitespace() {
        j += 1;
    }
    if j < sb.len() && sb[j] == b'(' {
        Some(name)
    } else {
        None
    }
}

/// `(cfg attributes, cfg! expressions)` naming [`FEATURE`] in one source file.
///
/// The predicate is read from the ORIGINAL text between the matched delimiters
/// (the stripped copy has blanked the feature name, which lives in a string
/// literal), while the *structure* — where an attribute starts and ends — comes
/// from the stripped copy, so nothing inside a comment can open one.
///
/// Everything below indexes BYTES, never `&str`. The engine's sources are full
/// of em-dashes and arrows in comments, and an offset landing mid-codepoint
/// panics `&str` slicing — which is how the first draft of this file went red
/// on the real tree while every fixture passed. `strip_comments_and_strings`
/// blanks byte-for-byte, so the two buffers share one offset space and every
/// offset used here is at an ASCII delimiter.
fn count_in_source(src: &str) -> (usize, usize) {
    let stripped = strip_comments_and_strings(src);
    let sb = stripped.as_bytes();
    let ob = src.as_bytes();
    let feat = FEATURE.as_bytes();
    let (mut attrs, mut macros) = (0usize, 0usize);

    let mut i = 0usize;
    while i < sb.len() {
        if sb[i] == b'#' {
            // `#[` or `#![`
            let mut j = i + 1;
            if j < sb.len() && sb[j] == b'!' {
                j += 1;
            }
            if j < sb.len()
                && sb[j] == b'['
                && matches!(attr_head(sb, j), Some(b"cfg") | Some(b"cfg_attr"))
            {
                let close = matching_close(sb, j, b"[(", b"])");
                if bytes_contain(&ob[j..=close.min(ob.len() - 1)], feat) {
                    attrs += 1;
                }
                i = close + 1;
                continue;
            }
        }
        // `cfg!(` — must not be preceded by an identifier character, so
        // `some_cfg!(...)` is not a match.
        if sb[i..].starts_with(b"cfg!")
            && (i == 0 || !(sb[i - 1].is_ascii_alphanumeric() || sb[i - 1] == b'_'))
        {
            let mut j = i + 4;
            while j < sb.len() && sb[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < sb.len() && sb[j] == b'(' {
                let close = matching_close(sb, j, b"(", b")");
                if bytes_contain(&ob[j..=close.min(ob.len() - 1)], feat) {
                    macros += 1;
                }
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    (attrs, macros)
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    attrs: usize,
    macros: usize,
    files: usize,
}

impl Counts {
    fn total(&self) -> usize {
        self.attrs + self.macros
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `.rs` under `dir`, sorted. `target/` is skipped: it is build output,
/// and counting generated code would swamp the figure this governs.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Count every `.rs` under `dir` for which `keep` returns true.
fn scan(dir: &Path, keep: &dyn Fn(&Path) -> bool) -> Counts {
    let mut c = Counts::default();
    for path in rust_sources(dir) {
        if !keep(&path) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        c.files += 1;
        let (a, m) = count_in_source(&src);
        c.attrs += a;
        c.macros += m;
    }
    c
}

fn model_counts() -> Counts {
    scan(&repo_root().join("crates/core/src"), &|_| true)
}

fn harness_counts() -> Counts {
    let root = repo_root();
    let model = root.join("crates/core/src");
    scan(&root.join("crates"), &|p| !p.starts_with(&model))
}

// ---------------------------------------------------------------------------
// The gate.
// ---------------------------------------------------------------------------

/// THE ratchet predicate — one definition, used by the gate below and by the
/// negative control. Deliberately a named function rather than an inline
/// comparison in each `assert!`: a negative control that re-types the
/// comparison proves only that `<=` works, not that the gate's own rule
/// rejects anything.
fn within_ceiling(total: usize, ceiling: usize) -> bool {
    total <= ceiling
}

#[test]
fn the_event_scheduler_cfg_surface_only_falls() {
    let model = model_counts();
    let harness = harness_counts();

    assert!(
        within_ceiling(model.total(), MAX_MODEL_SITES),
        "event-scheduler conditional-compilation sites in crates/core/src rose to {} \
         (ceiling {MAX_MODEL_SITES}): {} cfg/cfg_attr attributes + {} cfg!() expressions over \
         {} scanned files. Every site is a place a peripheral author has to be correct in two worlds \
         while the build compiles one — the surface went 19 -> 208 in eight weeks while \
         Cargo.toml still promised the feature would be flipped unconditional and deleted. \
         If the new site is a step that ENDS the feature, say so in this commit and raise \
         MAX_MODEL_SITES. If it is another permanent fork, say that instead — it is the same \
         edit, and the difference is the whole decision.",
        model.total(),
        model.attrs,
        model.macros,
        model.files
    );

    assert!(
        within_ceiling(harness.total(), MAX_HARNESS_SITES),
        "event-scheduler conditional-compilation sites outside crates/core/src rose to {} \
         (ceiling {MAX_HARNESS_SITES}): {} attributes + {} cfg!() over {} scanned files. A whole test \
         file behind `#![cfg(feature = \"event-scheduler\")]` compiles to nothing and reports \
         `0 passed` as a pass — see `no_vacuous_test_targets`. Raise this only with the reason \
         in the commit.",
        harness.total(),
        harness.attrs,
        harness.macros,
        harness.files
    );
}

/// The scan must be able to see the code it governs. Without this the assertions
/// above pass on an empty walk — the failure this repo keeps finding, where a
/// gate reports success having read nothing.
#[test]
fn the_scan_is_not_vacuous() {
    let model = model_counts();
    let harness = harness_counts();
    assert!(
        model.files > 400,
        "only {} .rs files scanned under crates/core/src; the walk is not reaching the engine",
        model.files
    );
    assert!(
        harness.files > 100,
        "only {} .rs files scanned outside crates/core/src; the walk is not reaching crates/",
        harness.files
    );
    assert!(
        model.attrs > 0 && model.macros > 0,
        "found {} attributes and {} cfg!() naming `{FEATURE}` across {} files — both forms \
         exist in this tree, so a zero means the matcher stopped matching and both ceilings \
         above are meaningless",
        model.attrs,
        model.macros,
        model.files
    );
}

// ---------------------------------------------------------------------------
// The counter's definition, pinned. These are what make the number above mean
// something specific rather than "whatever a regex happened to hit".
// ---------------------------------------------------------------------------

#[test]
fn every_counted_form_is_counted_once() {
    // Positive control: one of each shape this file claims to count.
    let src = r####"
#![cfg(feature = "event-scheduler")]
#[cfg(feature = "event-scheduler")]
fn a() {}
#[cfg(not(feature = "event-scheduler"))]
fn b() {}
#[cfg(all(feature = "event-scheduler", not(debug_assertions)))]
fn c() {}
#[cfg_attr(feature = "event-scheduler", allow(dead_code))]
fn d() {}
#[cfg(
    any(
        feature = "event-scheduler",
        feature = "jit"
    )
)]
fn e() {}
fn f() -> bool { cfg!(feature = "event-scheduler") }
fn g() -> bool { cfg!(all(feature = "event-scheduler", feature = "jit")) }
"####;
    assert_eq!(
        count_in_source(src),
        (6, 2),
        "the six attribute forms (inner, plain, negated, all(), cfg_attr, multi-line) and both \
         cfg!() forms must each count exactly once"
    );
}

#[test]
fn unrelated_cfgs_are_not_counted() {
    // Negative control on the predicate: a cfg that does not name the feature
    // is not this feature's surface, however scheduler-ish it reads.
    let src = r####"
#[cfg(feature = "jit")]
fn a() {}
#[cfg(test)]
fn b() {}
#[cfg(target_arch = "wasm32")]
fn c() {}
fn d() -> bool { cfg!(feature = "scheduler") }
fn e() -> bool { cfg!(debug_assertions) }
"####;
    assert_eq!(count_in_source(src), (0, 0));
}

#[test]
fn prose_about_the_feature_is_not_a_site() {
    // THE MISCOUNT THIS GATE EXISTS NOT TO REPEAT. Naive grep reports 217 model
    // sites against a true 208; the nine extras are all of these shapes.
    let src = r####"
//! `crates/core/tests/board_batch_width.rs` opens with
//! `#![cfg(feature = "event-scheduler")]`, so without the feature the whole
//! file compiles to nothing.

// Gated on `event-scheduler`: `uses_scheduler()` is
// `cfg!(feature = "event-scheduler") && clock.is_some()`.

/* #[cfg(feature = "event-scheduler")] left here during a bisect
   /* with a nested comment inside it */ */

fn msg() {
    assert!(false, "found no #![cfg(feature = \"event-scheduler\")] tests");
    let _ = r#"#[cfg(feature = "event-scheduler")]"#;
}
"####;
    assert_eq!(
        count_in_source(src),
        (0, 0),
        "doc comments, line comments, nested block comments, escaped assertion strings and raw \
         string fixtures all merely MENTION the feature; none of them asks the compiler for a \
         world"
    );
}

#[test]
fn non_ascii_prose_does_not_derail_the_scan() {
    // THE BUG THIS FILE SHIPPED FIRST. The engine's comments are full of
    // em-dashes, arrows and ⚠️; the original draft sliced `&str` at byte
    // offsets computed over the stripped copy and panicked mid-codepoint on the
    // real tree while every ASCII fixture passed. A counter that panics is not
    // a green gate, but it is not a red one for the reason it claims either.
    let src = "//! ⚠️ the walk — see `#[cfg(feature = \"event-scheduler\")]` → below\n\
               #[cfg(feature = \"event-scheduler\")] // ← counted\n\
               fn a() {}\n\
               /// Δt — cadence\n\
               fn b() -> bool { cfg!(feature = \"event-scheduler\") }\n";
    assert_eq!(count_in_source(src), (1, 1));
}

/// This file names the feature dozens of times, all of it in prose and
/// fixtures. If [`strip_comments_and_strings`] ever regressed, this file would
/// start counting itself and the ceilings would drift by its own edits — the
/// reason `downcast_ratchet` has to exclude itself by path. Proving the
/// property is better than excluding by path, because the property also covers
/// every other file that merely discusses the feature.
#[test]
fn this_file_contributes_no_sites() {
    let me = repo_root().join("crates/core/src/tests/event_scheduler_cfg_ratchet.rs");
    let src = std::fs::read_to_string(&me).expect("read this file");
    assert!(
        src.contains(FEATURE),
        "fixture check is vacuous: this file no longer mentions `{FEATURE}`"
    );
    assert_eq!(
        count_in_source(&src),
        (0, 0),
        "this ratchet counted ITSELF; its ceilings now move when its own comments are edited"
    );
}

// ---------------------------------------------------------------------------
// NEGATIVE CONTROL
//
// A gate nobody has watched fail is the defect one level up. This writes a
// scratch fixture into a temp directory and runs the REAL directory walk over
// it — not the pure counter — so the file discovery, the read and the ceiling
// comparison are all exercised on a surface that is one site too large.
// ---------------------------------------------------------------------------

#[test]
fn a_new_site_makes_the_gate_go_red() {
    let dir = std::env::temp_dir().join(format!(
        "labwired-es-cfg-ratchet-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("nested")).expect("scratch dir");

    // A file that merely talks about the feature: the walk must find it and
    // count nothing, or the "rise" below would not be attributable.
    std::fs::write(
        dir.join("prose.rs"),
        "//! mentions #[cfg(feature = \"event-scheduler\")] in prose only\nfn a() {}\n",
    )
    .expect("write prose fixture");
    let baseline = scan(&dir, &|_| true);
    assert_eq!(
        (baseline.attrs, baseline.macros, baseline.files),
        (0, 0, 1),
        "scratch baseline must be one file with zero sites, got {baseline:?}"
    );

    // Now add one real site, of each counted form, in a nested directory.
    std::fs::write(
        dir.join("nested/regression.rs"),
        "#[cfg(feature = \"event-scheduler\")]\nfn a() {}\nfn b() -> bool { cfg!(feature = \"event-scheduler\") }\n",
    )
    .expect("write regression fixture");
    let risen = scan(&dir, &|_| true);

    assert_eq!(
        (risen.attrs, risen.macros, risen.files),
        (1, 1, 2),
        "the walk did not pick up the added sites, so the ceilings above cannot go red for a \
         real one either; got {risen:?}"
    );
    assert!(
        risen.total() > baseline.total(),
        "adding a gated item did not raise the count"
    );

    // And the gate's OWN predicate — the same `within_ceiling` the two
    // assertions above call, not a re-typed comparison — must reject a surface
    // that is over by exactly the sites this fixture added, while still
    // accepting the ceiling itself.
    assert!(
        within_ceiling(MAX_MODEL_SITES, MAX_MODEL_SITES),
        "the ratchet predicate rejected a count exactly AT its ceiling; today's tree would be \
         red for no change at all"
    );
    assert!(
        !within_ceiling(MAX_MODEL_SITES + risen.total(), MAX_MODEL_SITES),
        "the ratchet predicate accepted a count above its own ceiling — the gate cannot go red"
    );
    assert!(
        !within_ceiling(MAX_HARNESS_SITES + risen.total(), MAX_HARNESS_SITES),
        "the harness ratchet predicate accepted a count above its own ceiling"
    );

    std::fs::remove_dir_all(&dir).expect("clean up scratch dir");
}
