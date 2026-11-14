// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `board_io` describes PINS. `external_devices:` declares PARTS. No display
//! resolution path may confuse the two again.
//!
//! # The duplicate that caused all of it
//!
//! [`labwired_config::SystemManifest`] has two surfaces that can describe the
//! same physical part. `external_devices:` says what a part IS and what it is
//! wired to — it is what attaches a real model to a bus. `board_io:` describes a
//! contact asserting a level, which is a genuinely different thing (a button, an
//! LED, a PIR or hall output), except that it also carries `device_type:` and
//! `i2c_address:` — restating what `external_devices` already said.
//!
//! Nothing forced the two to agree, and the engine only reads one of them. It
//! attaches the panel from `external_devices:`, while every wasm framebuffer
//! accessor searched `board_io` for a matching `device_type`. So a lab that
//! declared its display correctly in ONE of the two got a panel that was fully
//! driven, painting every frame, and completely invisible. Declared in both, it
//! painted. That is not a bug about a model or a chip — it is a bug about which
//! surface the author happened to write in.
//!
//! # What this guards
//!
//! Display resolution now goes through the attached-device seam, which sees a
//! device attached by ANY route, so `board_io.device_type` has no consumer left
//! on that path. The field is not deleted — 47 files read `board_io` and the
//! rest of it is load-bearing — it is made **non-load-bearing**, which is the
//! win. This test is what stops it becoming load-bearing again in six months,
//! quietly, in one accessor, for one model.
//!
//! Deliberately a SOURCE-level check. The behavioural gates
//! (`display_one_door.rs`) prove the door answers today; only reading the source
//! can prove that no future path resolves a display the old way — a behavioural
//! test passes just as happily when a second, `board_io`-keyed lookup is added
//! beside the working one, which is exactly how the fork appeared the first time.

use std::path::PathBuf;

fn wasm_inspect_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../wasm/src/inspect.rs")
        .canonicalize()
        .expect("crates/wasm/src/inspect.rs exists");
    std::fs::read_to_string(path).expect("read crates/wasm/src/inspect.rs")
}

/// The display accessors the UI calls. Every one of these exists on `main` too,
/// which is what makes this guard's red a REAL one: run it against unmodified
/// `main` and it fails naming `get_ssd1306_framebuffer` and its six siblings,
/// each of which opens by matching a `board_io` row's `device_type`. It does not
/// fail because its subject is missing.
///
/// That distinction is the whole point. A guard whose only pre-fix failure is
/// "the function I guard does not exist yet" passes vacuously the day someone
/// deletes the thing it guards and updates the guard to match. These names are
/// the stable, pre-existing surface; [`NEW_RESOLUTION_FNS`] holds the ones this
/// change introduced, checked separately so their absence can never be mistaken
/// for the defect.
const SHIPPED_DISPLAY_ACCESSORS: &[&str] = &[
    "pub fn get_ssd1306_framebuffer(",
    "pub fn get_sh1107_framebuffer(",
    "pub fn get_ili9341_framebuffer(",
    "pub fn get_pcd8544_framebuffer(",
    "pub fn get_ssd1680_framebuffer(",
    "pub fn get_uc8151d_framebuffer(",
    "pub fn get_led_matrix_framebuffer(",
    "pub fn get_lcd1602_text(",
    "pub fn get_ssd1680_refresh_generation(",
    "pub fn get_uc8151d_refresh_generation(",
];

/// The resolution helpers this change introduced. Absent on `main` by
/// definition, so a missing one here is reported as "not applicable on this
/// tree", never as the defect.
const NEW_RESOLUTION_FNS: &[&str] = &[
    "fn display_artifact(",
    "fn panel_artifact(",
    "fn panel_bytes(",
    "fn panel_meta(",
    "fn refresh_generation(",
    "pub fn get_display(",
];

/// The body of `signature`, or `None` when this tree does not define it.
/// Comments are stripped first: this is about what the code READS, and a doc
/// comment that mentions the field by name is documentation, not a dependency.
fn body_of_opt(source: &str, signature: &str) -> Option<String> {
    source.find(signature)?;
    Some(body_of(source, signature))
}

/// The body of `signature`, brace-matched from its opening `{`.
fn body_of(source: &str, signature: &str) -> String {
    let start = source.find(signature).unwrap_or_else(|| {
        panic!(
            "crates/wasm/src/inspect.rs no longer defines `{signature}` — \
             if it was renamed, rename it here; if it was deleted, delete it here. \
             A guard that silently stops covering its subject is worse than no guard."
        )
    });
    let open = source[start..]
        .find('{')
        .map(|i| start + i)
        .expect("function has a body");
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut end = open;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    source[open..=end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// No display path may decide what a device IS from `board_io`.
///
/// **This test fails on unmodified `main`, for the real reason.** Every one of
/// the ten accessors below exists there and opens by matching a `board_io`
/// row's `device_type`, so `main` reports:
///
/// ```text
/// `pub fn get_ssd1306_framebuffer(` reads `device_type`. …
/// ```
///
/// which is exactly the defect: a display's identity taken from a `board_io`
/// row restating what `external_devices:` already declared, on a surface that
/// only reads one of the two.
#[test]
fn no_display_path_resolves_a_device_by_board_io_device_type() {
    let source = wasm_inspect_source();
    let subjects = SHIPPED_DISPLAY_ACCESSORS
        .iter()
        .map(|s| (*s, body_of(&source, s)))
        // A helper this tree does not have is not evidence either way.
        .chain(
            NEW_RESOLUTION_FNS
                .iter()
                .filter_map(|s| body_of_opt(&source, s).map(|b| (*s, b))),
        );
    for (signature, body) in subjects {
        assert!(
            !body.contains("device_type"),
            "`{signature}` reads `device_type`. A display's identity comes from the model's \
             own artifact (`meta.format`), never from a `board_io` row restating what \
             `external_devices:` already declared — that duplicate is why a correctly \
             declared, actively painting panel could be invisible to the browser. If a new \
             display genuinely cannot be resolved without it, that is a missing \
             `DeviceEvidence` impl, not a reason to reopen this."
        );
    }
}

/// And the one door itself must not need `board_io` at all.
///
/// The per-model shims may still consult a binding for PLACEMENT (which
/// controller, which address) when the manifest join could not name a
/// programmatically attached model. The door may not: a display declared in
/// `external_devices:` alone — Ryan's shape, and the shape that never worked —
/// has no row for it to read.
#[test]
fn the_one_door_resolves_without_a_board_io_row() {
    let source = wasm_inspect_source();
    // Deliberately a hard failure, not a skip: a guard that goes quiet when its
    // subject disappears is how the thing it guards gets deleted unnoticed. On a
    // tree that predates the door this is a "not implemented here" red, NOT a
    // demonstration of the board_io defect — that one is
    // `no_display_path_resolves_a_device_by_board_io_device_type`, which fails
    // on unmodified main naming `get_ssd1306_framebuffer`.
    assert!(
        source.contains("pub fn get_display("),
        "crates/wasm/src/inspect.rs defines no `get_display` — there is no one door on this \
         tree. If you are running this against a checkout that predates it, that is expected \
         and this test is not the defect demonstration; see \
         `no_display_path_resolves_a_device_by_board_io_device_type`. If the door was deleted, \
         restore it: without it a display declared only in `external_devices:` cannot render."
    );
    let door = body_of(&source, "pub fn get_display(");
    assert!(
        !door.contains("board_io"),
        "`get_display` reads `board_io`. The door's whole point is that a display \
         author writes ONE `external_devices:` entry and it paints — in the CLI, in the \
         browser, in `inspect`, in the 3D view — with no second declaration anywhere."
    );
    // Not vacuous: the door must actually go through the seam, or an empty
    // function would satisfy the assertion above.
    assert!(
        door.contains("display_artifact"),
        "`get_display` must resolve through the attached-device seam"
    );
}
