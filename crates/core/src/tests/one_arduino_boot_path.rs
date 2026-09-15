// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ONE Arduino-ESP32 boot path — a guard, not a convention.
//!
//! ## What went wrong
//!
//! `crates/wasm/src/install.rs` grew its own hand-maintained copy of the
//! Arduino-ESP32 bootstrap alongside the shared
//! [`crate::system::xtensa::install_arduino_esp32_profile`] the CLI uses. Two
//! lists that must agree, with nothing making them agree, so they drifted: at
//! the commit this test was written against, the browser named **29** boot
//! symbols the shared profile did not carry — the whole newlib `_lock_*`
//! family, the `esp_log*` family, the `esp_flash_*` bring-up set,
//! `spi_flash_*_lock`, `do_core_init`, `do_secondary_init`.
//!
//! (An eyeball grep put that number at 25. It was wrong because it did not
//! strip comments, so prose in the profile quoting a symbol counted as the
//! profile owning it. The scan below strips `//` first, which is why it finds
//! four more. Measure with the thing that will keep measuring.)
//!
//! ## Why a comment could not have caught it
//!
//! The first attempt at this fix (branch `feat/one-arduino-boot-path`, never
//! opened as a PR) deleted the copy and left a comment reading "do not start a
//! third copy here". That is a sticky note. It cannot fail. The defect being
//! fixed is *silent divergence*, and the fix for silent divergence is a thing
//! that goes red — a thunk has no failure mode, it just quietly succeeds, so
//! two boot paths can answer differently for the same firmware forever without
//! one test noticing.
//!
//! ## What this asserts
//!
//! Every C-symbol-shaped string literal in the browser's installer must be one
//! the shared profile already owns, **or** be named in [`KNOWN_FORK`] with a
//! written reason. That list is content-keyed and shrink-only: a thirtieth
//! forked symbol turns this red, and an entry that stops being forked must be
//! deleted rather than left to rot.
//!
//! ## Why an allowlist and not a deletion
//!
//! The 29 entries below are not hypothetical debt — they are thunks the
//! browser installs today. Deleting them changes what a lab renders, and
//! nothing in this repo runs in a browser to tell you whether a lab still
//! paints afterwards.
//!
//! (An earlier version of this comment said `crates/wasm/tests/` held **zero**
//! test files. That was wrong: it holds `motor_states.rs`, and the crate has
//! 32 more host-side tests behind `cfg(all(test, not(target_arch = "wasm32")))`.
//! They compile the browser crate natively, which is real coverage — it is just
//! not a browser, so the conclusion stands on the corrected fact rather than
//! the overstated one.) The earlier attempt
//! at this fix deleted all of them at once with no lab run and no gate; that is
//! the mistake this file exists to avoid repeating. Closing an entry means
//! running the lab that exercises it, not arguing that it looks safe.
//!
//! What the allowlist buys immediately: the fork can no longer **grow**, and it
//! is counted in one place instead of being invisible.
//!
//! Reading the other side's *source* is deliberate. A test that asked the
//! browser to describe itself would be a mirror — it would pass for any pair of
//! lists so long as both halves of the same crate agreed, which is exactly the
//! condition that was broken.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

const PROFILE: &str = "crates/core/src/system/xtensa/arduino_esp32_profile.rs";
const BROWSER: &str = "crates/wasm/src/install.rs";

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel))
        .unwrap_or_else(|e| panic!("{rel} must be readable for this guard to mean anything: {e}"))
}

/// String literals shaped like a C identifier, with `//` comments stripped
/// first so prose quoting a symbol name ("…next = \"lock\"…") is not mistaken
/// for a declaration. Block comments are not used for symbol lists in either
/// file; if that changes, this scan gets stricter, not looser.
fn symbol_literals(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        let bytes: Vec<char> = code.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == '"' {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != '"' {
                    j += 1;
                }
                if j < bytes.len() {
                    let lit: String = bytes[start..j].iter().collect();
                    if is_c_identifier(&lit) {
                        out.insert(lit);
                    }
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
    }
    out
}

fn is_c_identifier(s: &str) -> bool {
    s.len() >= 3
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Boot symbols the browser still names on its own, each with the reason it
/// has not been closed yet. **This list may only ever get SHORTER.**
///
/// Closing one is not a code edit — it is: route that symbol through the shared
/// profile (or establish the CLI was right to drop it), run a browser lab that
/// exercises the path, then delete the entry.
const KNOWN_FORK: &[(&str, &str)] = &[
    // newlib locking. The CLI profile carries only the `__retarget_lock_*_recursive`
    // family, which are DIFFERENT symbols; these ten are the plain newlib ones.
    // With real FreeRTOS running on the emulated heap the CLI evidently does not
    // need them stubbed — but "evidently" is an argument, and un-stubbing a lock
    // is how you get a browser deadlock nobody can reproduce on the CLI.
    (
        "_lock_acquire",
        "newlib lock family; CLI runs without it, browser unproven",
    ),
    ("_lock_acquire_recursive", "newlib lock family"),
    ("_lock_close", "newlib lock family"),
    ("_lock_close_recursive", "newlib lock family"),
    ("_lock_init", "newlib lock family"),
    ("_lock_init_recursive", "newlib lock family"),
    ("_lock_release", "newlib lock family"),
    ("_lock_release_recursive", "newlib lock family"),
    ("_lock_try_acquire", "newlib lock family"),
    ("_lock_try_acquire_recursive", "newlib lock family"),
    // ESP-IDF core init. Stubbing these skips real init the CLI now runs.
    ("do_core_init", "IDF core init; CLI runs the real one"),
    (
        "do_secondary_init",
        "IDF secondary init; CLI runs the real one",
    ),
    // Flash bring-up. The profile models flash via its own chip-driver stubs
    // (spi_flash_chip_generic_*, spi_flash_hal_*), which the browser lacks, so
    // these are not a like-for-like swap and must be moved as a set.
    (
        "esp_flash_app_disable_protect",
        "flash bring-up; profile models flash differently",
    ),
    ("esp_flash_app_init", "flash bring-up"),
    ("esp_flash_chip_driver_initialized", "flash bring-up"),
    ("esp_flash_init", "flash bring-up"),
    ("esp_flash_init_default_chip", "flash bring-up"),
    ("esp_flash_init_main", "flash bring-up"),
    ("esp_flash_read_chip_id", "flash bring-up"),
    ("esp_partition_main_flash_region_safe", "flash bring-up"),
    ("spi_flash_init_lock", "flash locking"),
    ("spi_flash_op_lock", "flash locking"),
    ("spi_flash_op_unlock", "flash locking"),
    // ESP logging. This is the divergence with a user-visible shape: the browser
    // nops `esp_log_writev`, the CLI does not, so an ESP_LOGI() can reach serial
    // in `labwired test` and not in the lab. NOT yet confirmed by running it —
    // see the PR. Do not repeat that claim as fact until someone has.
    (
        "esp_log_early_timestamp",
        "ESP log family; browser nops, CLI does not",
    ),
    ("esp_log_impl_lock", "ESP log family"),
    ("esp_log_impl_lock_timeout", "ESP log family"),
    ("esp_log_impl_unlock", "ESP log family"),
    ("esp_log_timestamp", "ESP log family"),
    (
        "esp_log_writev",
        "ESP log family; the one with a visible symptom",
    ),
];

#[test]
fn the_browser_names_no_boot_symbol_outside_the_known_fork() {
    let root = repo_root();
    let profile = symbol_literals(&read(&root, PROFILE));
    let browser = symbol_literals(&read(&root, BROWSER));
    let known: BTreeSet<&str> = KNOWN_FORK.iter().map(|(s, _)| *s).collect();

    let unlisted: Vec<&String> = browser
        .difference(&profile)
        .filter(|s| !known.contains(s.as_str()))
        .collect();
    assert!(
        unlisted.is_empty(),
        "{} names {} boot symbol(s) that neither {} owns nor KNOWN_FORK documents. The \
         browser is growing a SECOND Arduino-ESP32 boot path. A thunk silently succeeds, so \
         the two paths can answer differently for the same firmware and nothing goes red.\n\n\
         Add the symbol to the shared profile (where thunk_debt_only_falls counts it), or \
         delete it from the browser. Do NOT extend KNOWN_FORK — that list only shrinks.\n\n\
         Undocumented forked symbols:\n  {}",
        BROWSER,
        unlisted.len(),
        PROFILE,
        unlisted
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn the_known_fork_only_shrinks() {
    // An entry that is no longer forked must be DELETED, not left behind. A
    // stale allowlist quietly re-authorises a fork someone already closed.
    let root = repo_root();
    let profile = symbol_literals(&read(&root, PROFILE));
    let browser = symbol_literals(&read(&root, BROWSER));

    let stale: Vec<&str> = KNOWN_FORK
        .iter()
        .map(|(s, _)| *s)
        .filter(|s| !browser.contains(*s) || profile.contains(*s))
        .collect();
    assert!(
        stale.is_empty(),
        "KNOWN_FORK lists {} symbol(s) that are no longer forked. Good — delete them from \
         the list; it is shrink-only and must not carry entries that no longer apply:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
    assert!(
        KNOWN_FORK.len() <= 29,
        "KNOWN_FORK has grown to {}. It was 29 when this guard landed and may only fall.",
        KNOWN_FORK.len()
    );
}

// NOTE: there is deliberately no "the browser delegates to the shared profile"
// assertion yet — on this commit it does not, and a test asserting an end state
// nobody has reached is just a red suite. `the_known_fork_only_shrinks` covers
// the hole it would have plugged: gutting the installer makes all 29 entries
// stale and turns that test red. Add the delegation assertion in the commit
// that actually routes the browser through the profile.

#[test]
fn the_scan_is_not_vacuous() {
    // Every way this scan can quietly measure nothing, asserted against.
    let root = repo_root();
    let profile_src = read(&root, PROFILE);
    let browser_src = read(&root, BROWSER);
    let profile = symbol_literals(&profile_src);

    assert!(
        profile.len() >= 50,
        "the shared profile should own dozens of symbols; found {}. Either the scan broke \
         or the profile moved, and in both cases the fork guard above is now vacuous.",
        profile.len()
    );
    for sentinel in ["esp_clk_init", "app_main", "pxCurrentTCB"] {
        assert!(
            profile_src.contains(sentinel),
            "sentinel {sentinel} missing from {PROFILE} — the scan is reading the wrong file"
        );
    }
    assert!(
        browser_src.len() > 1_000,
        "{BROWSER} is {} bytes; too small to be the real installer",
        browser_src.len()
    );
    // The extractor must actually reject prose. If this ever passes trivially,
    // the difference test above stops being able to tell a declaration from a
    // sentence that mentions a symbol.
    let prose = symbol_literals("let x = \"esp_log_writev\"; // \"do_core_init\" in a comment\n");
    assert!(
        prose.contains("esp_log_writev") && !prose.contains("do_core_init"),
        "the literal extractor must read code and ignore comments; got {prose:?}"
    );
}
