// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The runtime-downcast count may not grow.
//!
//! Remediation row 6.5 wants capability traits in place of `as_any()` +
//! `downcast_ref`. That is a long job: the 193 call sites spread across ~60
//! distinct concrete types, with no dominant family — the largest is `Uart` at
//! seven — so every conversion is its own decision about what the capability
//! actually is. It is not something a sweep finishes.
//!
//! What is finishable now is stopping the number going the wrong way, and that
//! is not hypothetical. **The row was written when there were 135 downcasts.
//! By the time it was re-derived there were 193** — a 43% increase, added by
//! ordinary work while the row sat queued and nobody was counting. A refactor
//! whose target grows faster than it shrinks never lands.
//!
//! So this is a ratchet, the shape used by `chip_pins_ratchet`,
//! `undecoded_register_ratchet` and `exhaustive-deps-baseline.json`: the counts
//! are committed, a rise fails, and a fall must be recorded so the ceiling
//! comes down with it.
//!
//! ## What it does NOT do
//!
//! It does not judge whether any individual downcast is justified — some are
//! (a test reaching into a concrete model), and some are the design debt the
//! row is about. Distinguishing them is exactly the work the row asks for. A
//! count cannot do it, and pretending otherwise would make this gate an excuse
//! rather than a floor.

use std::path::{Path, PathBuf};

/// Committed ceilings. LOWER these when a conversion lands; the test fails if
/// you do not, so a shrink cannot go unrecorded and quietly leave headroom for
/// the next regression.
/// 193 → 194 / 207 → 208: `tests/esp32s3_lcd_i80_pixels.rs` reaches through
/// `bus.peripherals[..].dev` to the concrete `Esp32s3LcdCam` to assert that the
/// kit binds the parallel panel to LCD_CAM and not only to the GPIO observer.
/// That is the "a test reaching into a concrete model" case the module doc
/// above names as justified — the alternative is a public accessor that exists
/// solely so one test need not downcast, which is worse design, not less debt.
///
/// 194 → 193 / 208 → 207: clock-gate resolution stopped downcasting to
/// `rcc::Rcc` and asks `Peripheral::clock_gate_reg_offset` instead. The
/// downcast was not merely debt, it was a correctness ceiling: it answered
/// `None` for any clock controller that is not an STM32 RCC, so an EFR32's CMU
/// could declare `clock:` gates that silently never resolved.
const MAX_AS_ANY: usize = 193;
const MAX_DOWNCAST_REF: usize = 207;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `.rs` under the crate source directories this ratchet governs.
///
/// `crates/` only, and deliberately: `examples/` is firmware built for other
/// architectures and holds no bus plumbing, so counting it would make the
/// number move for reasons unrelated to the design debt.
fn rust_sources(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // `target/` is build output; counting it would swamp the real
                // figure with generated code and vendored dependencies.
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
    walk(&root.join("crates"), &mut out);
    out.sort();
    out
}

struct Counts {
    as_any: usize,
    downcast_ref: usize,
    files_scanned: usize,
}

fn count() -> Counts {
    let root = repo_root();
    let mut as_any = 0;
    let mut downcast_ref = 0;
    let mut files_scanned = 0;
    for path in rust_sources(&root) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        // This file's own doc comment names both patterns, so it would count
        // itself and drift by its own edits.
        if path.ends_with("tests/downcast_ratchet.rs") {
            continue;
        }
        files_scanned += 1;
        as_any += src.matches("as_any()").count();
        downcast_ref += src.matches("downcast_ref").count();
    }
    Counts {
        as_any,
        downcast_ref,
        files_scanned,
    }
}

#[test]
fn the_downcast_count_only_shrinks() {
    let c = count();

    assert!(
        c.as_any <= MAX_AS_ANY,
        "as_any() call sites rose to {} (ceiling {MAX_AS_ANY}). This is remediation row 6.5's \
         debt growing, which is exactly what happened last time: the row was written at 135 and \
         re-derived at 193. Reach for the concrete type through a capability trait instead, or \
         raise MAX_AS_ANY in the same commit that explains why.",
        c.as_any
    );
    assert!(
        c.downcast_ref <= MAX_DOWNCAST_REF,
        "downcast_ref sites rose to {} (ceiling {MAX_DOWNCAST_REF}). See the as_any message.",
        c.downcast_ref
    );

    // A ceiling left above the real number is headroom for the next regression
    // to hide in, so a shrink must be recorded rather than banked.
    assert_eq!(
        c.as_any, MAX_AS_ANY,
        "as_any() is down to {} but MAX_AS_ANY is still {MAX_AS_ANY}. Lower it in the same commit \
         — slack in a ratchet silently re-admits what it just removed.",
        c.as_any
    );
    assert_eq!(
        c.downcast_ref, MAX_DOWNCAST_REF,
        "downcast_ref is down to {} but MAX_DOWNCAST_REF is still {MAX_DOWNCAST_REF}. Lower it.",
        c.downcast_ref
    );
}

/// The scan must be able to see the code it governs. Without this the two
/// assertions above pass on an empty walk — the failure mode this repo keeps
/// finding, where a gate reports success having read nothing.
#[test]
fn the_scan_is_not_vacuous() {
    let c = count();
    assert!(
        c.files_scanned > 500,
        "only {} .rs files scanned; the walk is not reaching crates/",
        c.files_scanned
    );
    assert!(
        c.as_any > 0 && c.downcast_ref > 0,
        "found no downcasts at all ({} as_any, {} downcast_ref) across {} files — the patterns \
         stopped matching, so the ceilings above are meaningless",
        c.as_any,
        c.downcast_ref,
        c.files_scanned
    );
}
