// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! THE CONTRACT: a peripheral that is excluded from the per-cycle walk must not
//! do work in `tick()`.
//!
//! # The bug class
//!
//! Two independent channels stop a peripheral's `tick()` ever being called, and
//! both are invisible in a default `cargo test`:
//!
//! 1. **Whole-walk deletion.** [`crate::bus::SystemBus::derive_walk_deletable`]
//!    deletes the legacy walk for the ENTIRE bus iff every peripheral reports
//!    `uses_scheduler() || !needs_legacy_walk()`. One model lying about itself
//!    starves every model on that bus.
//! 2. **Per-peripheral skip.** The walk loop returns `default()` for any
//!    `uses_scheduler()` model even when the walk runs. A model that claims
//!    scheduler mode without an event chain, a `sync_to`, or a matrix level
//!    export is starved regardless of channel 1.
//!
//! Both are gated on `--features event-scheduler`, which the browser/wasm crate
//! enables and every CLI lane in this repo does not. So the failure appears
//! only in the product, and only as a hang.
//!
//! # Why this gate is STATIC
//!
//! Runtime falsification does not work here and was tried. A register-level
//! fuzzer that pokes a model and diffs walk-on against walk-off misses this
//! entire class, because it never leaves the model in an *asserting* state: the
//! difference only exists while an interrupt level is up, which takes a
//! specific arming sequence the falsifier does not know. Three confirmed
//! defects (classic-ESP32 UART0, ESP32-C3 RMT, RP2040 I2C0) all survived it.
//!
//! A static scan asks the question that actually distinguishes them: does the
//! source say "I need no walk" while also containing walk work? That is
//! answerable by reading the code, needs no firmware, no feature flag and no
//! bus, and cannot be satisfied vacuously.
//!
//! # The three rules
//!
//! * **A.** An `impl Peripheral` whose `needs_legacy_walk()` is literally
//!   `false` must have a literally-default `tick()` / `tick_elapsed()`.
//!   Caught: ESP32-C3 RMT, RP2040 I2C0.
//! * **B.** An `impl Peripheral` that can report `uses_scheduler() == true`
//!   while having a non-default `tick()` must ALSO declare a delivery hook
//!   (`on_event`, `take_scheduled_events`, `sync_to`, `matrix_irq_sources_into`
//!   / `matrix_irq_sources`, or `tick_with_bus`) — otherwise the walk skip in
//!   channel 2 silently drops its work on the floor.
//! * **C.** No production source may hand-assign `legacy_walk_disabled = true`.
//!   Walk deletion is DERIVED (`recompute_walk_deletable`) or opted into per
//!   config (`manifest.walk_deleted`); a hand assert claims a property about
//!   peripherals it never inspects. Caught: classic ESP32
//!   (`system/xtensa/esp32.rs`), where the comment named `uart0` as migrated
//!   and `uart0` never was.
//!
//! # Conditional forms
//!
//! `needs_legacy_walk()` bodies that are the textual negation of the same
//! impl's `uses_scheduler()` body (`!self.scheduler_mode()`,
//! `!self.uses_scheduler()`) are self-consistent by construction: the model
//! leaves the walk exactly when it joins the scheduler, so channel 1's OR can
//! never starve it. Those are accepted automatically. Every OTHER conditional
//! shape needs an entry in [`CONDITIONAL_ALLOWLIST`] with a justification.
//!
//! # Allowlists shrink, never grow
//!
//! Each allowlist entry is re-checked: an entry that no longer violates fails
//! the gate, so a fixed model cannot leave a stale exemption behind for the
//! next offender to hide under.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// Frozen allowlists. Adding a line here is a deliberate act; read the rule
// docs above before you do it.
// ─────────────────────────────────────────────────────────────────────────────

/// Rule A exemptions: models that declare `needs_legacy_walk() == false` while
/// their `tick()` still does work.
///
/// EVERY entry is a live instance of the same defect class as the two fixed in
/// this PR, found by this gate on its first run. They are recorded rather than
/// fixed because each needs its own delivery-path design (an event chain, or a
/// per-fabric IRQ route) and its own behavioural proof — a blanket
/// `needs_legacy_walk() -> true` would trade the starvation for a whole-bus
/// throughput regression, which is the trade this codebase keeps making by
/// accident.
///
/// Format: `(path relative to crates/core/src, impl type, why it is still here)`.
const RULE_A_ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "peripherals/rp2040/usb.rs",
        "Rp2040Usb",
        "tick() runs host_poll() and emits USBCTRL_IRQ. Needs the same delay-1 \
         event chain as Rp2040I2c, plus a decision about host_poll cadence.",
    ),
    (
        "peripherals/nrf52/clock.rs",
        "Nrf52Clock",
        "tick() re-pends a held HFCLKSTARTED/LFCLKSTARTED level (irq: true).",
    ),
    (
        "peripherals/nrf52/ecb.rs",
        "Nrf52Ecb",
        "tick() latches ENDECB and pends the AES ECB line.",
    ),
    (
        "peripherals/nrf52/egu.rs",
        "Nrf52Egu",
        "tick() drains software-triggered EGU events into IRQs.",
    ),
    (
        "peripherals/nrf52/gpiote.rs",
        "Nrf52Gpiote",
        "tick() drains pending PORT/IN events into IRQs.",
    ),
    (
        "peripherals/nrf52/radio.rs",
        "Nrf52Radio",
        "tick() advances the TX/RX cycle countdown and fires ADDRESS/END.",
    ),
    (
        "peripherals/nrf52/serial_instance.rs",
        "Nrf52SerialInstance",
        "tick() delegates to the active TWIM/SPIM sub-model, both of which tick.",
    ),
    (
        "peripherals/nrf52/twim.rs",
        "Nrf52Twim",
        "tick() converts latched TWIM events into the instance IRQ.",
    ),
    (
        "peripherals/esp32s3/gpio.rs",
        "Esp32s3Gpio",
        "tick() advances an internal cycle counter used for edge timestamps; no \
         IRQ, but the counter is observable state mutated from the walk.",
    ),
];

/// Rule A/B conditional-shape exemptions: `needs_legacy_walk()` bodies that are
/// not literal and not the textual negation of the impl's own
/// `uses_scheduler()`.
const CONDITIONAL_ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "peripherals/bxcan.rs",
        "BxCan",
        "`self.bus_rx.is_some()`: the only thing tick() does is drain the CanBus \
         mpsc interconnect, so with no interconnect attached the tick is a \
         genuine structural no-op. State-dependent but exhaustive.",
    ),
    (
        "peripherals/declarative.rs",
        "GenericPeripheral",
        "`has_read_trigger() || has_write_trigger() || !inflight_events.is_empty()`: \
         a descriptor with no triggers and no seeded periodic events has an \
         inert tick. new() seeds the inflight set, so the predicate covers the \
         whole reachable space.",
    ),
    (
        "peripherals/nrf52/rng.rs",
        "Nrf52Rng",
        "`self.clock.is_none()` is the semantic negation of \
         `uses_scheduler() = self.clock.is_some()`; only the spelling differs.",
    ),
    (
        "peripherals/nrf52/wdt.rs",
        "Nrf52Wdt",
        "`self.clock.is_none()` is the semantic negation of \
         `uses_scheduler() = self.clock.is_some()`; only the spelling differs.",
    ),
];

/// Rule C exemptions: production sites that hand-assign
/// `legacy_walk_disabled = true`.
///
/// Marked `pending_external_fix` because a hand assert is never correct — it is
/// waiting on someone else's landing, not justified. Unlike the other two
/// lists, a stale entry here does NOT fail the gate: it is expected to
/// disappear from under us when that fix merges, and a cross-PR shrink check
/// would just make two correct branches conflict.
const RULE_C_PENDING_FIX: &[(&str, &str)] = &[(
    "system/xtensa/esp32.rs",
    "classic-ESP32 asserts walk deletion under a comment claiming uart0 was \
     migrated to the scheduler; Esp32Uart never was, so uart0's tx_fifo is \
     never drained and arduino-esp32 spins in uart_ll_write_txfifo. Fix in \
     flight on fix/esp32-uart-event-scheduler-starvation, which replaces the \
     assert with recompute_walk_deletable(). Delete this entry when it lands.",
)];

/// Delivery hooks that make a `uses_scheduler()` model's work observable.
const DELIVERY_HOOKS: &[&str] = &[
    "on_event",
    "take_scheduled_events",
    "sync_to",
    "matrix_irq_sources_into",
    "matrix_irq_sources",
    "tick_with_bus",
];

/// Bodies that count as a literally-default `tick()`.
const DEFAULT_TICK_BODIES: &[&str] = &[
    "PeripheralTickResult::default()",
    "crate::PeripheralTickResult::default()",
    "Default::default()",
];

// ─────────────────────────────────────────────────────────────────────────────
// Scanner
// ─────────────────────────────────────────────────────────────────────────────

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Replace comments and string literals with spaces so brace matching and
/// keyword search never trip over `"{"` or a doc comment. Length-preserving, so
/// byte offsets stay usable for line numbers.
fn blank_comments_and_strings(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = vec![b' '; b.len()];
    let mut i = 0;
    while i < b.len() {
        // Keep newlines so line numbers survive.
        if b[i] == b'\n' {
            out[i] = b'\n';
            i += 1;
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1;
            i += 2;
            while i + 1 < b.len() && depth > 0 {
                if b[i] == b'/' && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if b[i] == b'\n' {
                        out[i] = b'\n';
                    }
                    i += 1;
                }
            }
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                if i < b.len() && b[i] == b'\n' {
                    out[i] = b'\n';
                }
                i += 1;
            }
            i += 1;
        } else {
            out[i] = b[i];
            i += 1;
        }
    }
    String::from_utf8(out).expect("ascii-preserving blanking")
}

/// End index (exclusive) of the block whose opening `{` is at `open`.
fn block_end(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    for (i, &c) in b.iter().enumerate().skip(open) {
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Blank out every `#[cfg(test)] mod ... { .. }` body. Test scaffolding builds
/// deliberately unrealistic peripherals and buses; the contract is about
/// production models only.
fn blank_cfg_test_modules(s: &str) -> String {
    let mut out = s.to_string();
    let mut from = 0usize;
    while let Some(rel) = out[from..].find("#[cfg(test)]") {
        let at = from + rel;
        // Only a `mod` immediately after the attribute encloses a block.
        let rest = &out[at + "#[cfg(test)]".len()..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with("mod ") || trimmed.starts_with("pub mod ") {
            if let Some(open_rel) = rest.find('{') {
                let open = at + "#[cfg(test)]".len() + open_rel;
                if let Some(end) = block_end(&out, open) {
                    let blanked: String = out[open..=end]
                        .chars()
                        .map(|c| if c == '\n' { '\n' } else { ' ' })
                        .collect();
                    out.replace_range(open..=end, &blanked);
                    from = end;
                    continue;
                }
            }
        }
        from = at + "#[cfg(test)]".len();
    }
    out
}

/// One parsed `impl Peripheral for T` block.
struct PeripheralImpl {
    file: String,
    ty: String,
    /// Method name → single-line-normalised body.
    methods: Vec<(String, String)>,
}

impl PeripheralImpl {
    fn body(&self, name: &str) -> Option<&str> {
        self.methods
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_str())
    }
    fn has(&self, name: &str) -> bool {
        self.body(name).is_some()
    }
    /// True when `tick()` or `tick_elapsed()` is overridden with anything other
    /// than the trait default.
    fn does_walk_work(&self) -> bool {
        let tick_bad = self
            .body("tick")
            .is_some_and(|b| !DEFAULT_TICK_BODIES.contains(&b));
        let te_bad = self
            .body("tick_elapsed")
            .is_some_and(|b| !DEFAULT_TICK_BODIES.contains(&b) && b != "self.tick()");
        tick_bad || te_bad
    }
}

fn normalise(body: &str) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Find every `impl Peripheral for T { .. }` in production source.
fn parse_peripheral_impls() -> Vec<PeripheralImpl> {
    let root = src_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let mut impls = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // `src/tests/` is this crate's lib-test tree — scaffolding, not models.
        if rel.starts_with("tests/") || rel.ends_with("tests_main.rs") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("read source");
        let clean = blank_cfg_test_modules(&blank_comments_and_strings(&raw));
        let mut search = 0usize;
        while let Some(rel_at) = clean[search..].find("impl ") {
            let at = search + rel_at;
            search = at + 5;
            // `impl [<generics>] Peripheral for <Type> {`
            let tail = &clean[at..];
            let Some(for_at) = tail.find(" for ") else {
                continue;
            };
            let head = &tail[..for_at];
            if !head.contains("Peripheral") || head.contains('{') {
                continue;
            }
            // Reject `impl I2cDevice for`, `impl PeripheralExt for`, etc.
            let last_token = head.rsplit(|c: char| !c.is_alphanumeric() && c != '_');
            if last_token.into_iter().next() != Some("Peripheral") {
                continue;
            }
            let Some(open_rel) = tail.find('{') else {
                continue;
            };
            let open = at + open_rel;
            let Some(end) = block_end(&clean, open) else {
                continue;
            };
            let ty = tail[for_at + 5..open_rel].trim().to_string();
            let body = &clean[open + 1..end];

            let mut methods = Vec::new();
            let mut fsearch = 0usize;
            while let Some(fn_rel) = body[fsearch..].find("fn ") {
                let fat = fsearch + fn_rel;
                fsearch = fat + 3;
                let after = &body[fat + 3..];
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.is_empty() {
                    continue;
                }
                let Some(bopen_rel) = body[fat..].find('{') else {
                    continue;
                };
                let bopen = fat + bopen_rel;
                let Some(bend) = block_end(body, bopen) else {
                    continue;
                };
                methods.push((name, normalise(&body[bopen + 1..bend])));
                fsearch = bend;
            }
            impls.push(PeripheralImpl {
                file: rel.clone(),
                ty,
                methods,
            });
            search = end;
        }
    }
    impls
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule A + conditional shapes
// ─────────────────────────────────────────────────────────────────────────────

/// A peripheral that says it needs no walk must genuinely do nothing in the
/// walk. See the module docs: this is the whole-walk-deletion channel, and it
/// is how ESP32-C3 RMT and RP2040 I2C0 both went silent in the browser.
#[test]
fn walk_free_peripherals_have_a_default_tick() {
    let impls = parse_peripheral_impls();
    assert!(
        impls.len() > 100,
        "scanner found only {} `impl Peripheral` blocks — it is broken, and a \
         broken scanner is a vacuously green gate",
        impls.len()
    );

    let allow: BTreeSet<(&str, &str)> = RULE_A_ALLOWLIST.iter().map(|(f, t, _)| (*f, *t)).collect();
    let mut violations = Vec::new();
    let mut hit: BTreeSet<(String, String)> = BTreeSet::new();

    for i in &impls {
        let Some(nlw) = i.body("needs_legacy_walk") else {
            continue;
        };
        if nlw != "false" || !i.does_walk_work() {
            continue;
        }
        if allow.contains(&(i.file.as_str(), i.ty.as_str())) {
            hit.insert((i.file.clone(), i.ty.clone()));
            continue;
        }
        violations.push(format!(
            "  {}::{}\n      needs_legacy_walk() -> false, but tick() = {}",
            i.file,
            i.ty,
            i.body("tick")
                .or_else(|| i.body("tick_elapsed"))
                .unwrap_or("")
        ));
    }

    assert!(
        violations.is_empty(),
        "\nWALK-STARVATION CONTRACT (rule A) violated by {} model(s):\n{}\n\n\
         `needs_legacy_walk() == false` asserts that this model's \
         tick()/tick_elapsed() cannot change observable state in ANY reachable \
         firmware state. A tick() that pends an IRQ, drains a FIFO or advances a \
         counter is not that. Once every peripheral on a bus makes this claim, \
         SystemBus::derive_walk_deletable deletes the walk and the model is never \
         ticked again — under --features event-scheduler, which is what the \
         browser runs and no CLI lane here does.\n\n\
         Fix it by putting the model on the event scheduler (uses_scheduler() + \
         an event chain, a sync_to, or a matrix level export), NOT by flipping \
         needs_legacy_walk() to true — that un-deletes the whole bus's walk and \
         trades a hang for a slowdown.\n",
        violations.len(),
        violations.join("\n")
    );

    // Shrink-only: a fixed model must not leave its exemption behind.
    let stale: Vec<&str> = RULE_A_ALLOWLIST
        .iter()
        .filter(|(f, t, _)| !hit.contains(&(f.to_string(), t.to_string())))
        .map(|(t, _, _)| *t)
        .collect();
    assert!(
        stale.is_empty(),
        "RULE_A_ALLOWLIST has stale entries (these no longer violate — delete \
         the lines): {stale:?}"
    );
}

/// Conditional `needs_legacy_walk()` bodies must either be the textual negation
/// of the same impl's `uses_scheduler()` (self-consistent by construction) or
/// carry a written justification.
#[test]
fn conditional_walk_predicates_are_self_consistent_or_justified() {
    let impls = parse_peripheral_impls();
    let allow: BTreeSet<(&str, &str)> = CONDITIONAL_ALLOWLIST
        .iter()
        .map(|(f, t, _)| (*f, *t))
        .collect();
    let mut violations = Vec::new();
    let mut hit: BTreeSet<(String, String)> = BTreeSet::new();

    for i in &impls {
        let Some(nlw) = i.body("needs_legacy_walk") else {
            continue;
        };
        if nlw == "true" || nlw == "false" {
            continue;
        }
        let self_consistent = nlw == "!self.uses_scheduler()"
            || i.body("uses_scheduler")
                .is_some_and(|us| nlw == format!("!{us}"));
        if self_consistent {
            continue;
        }
        if allow.contains(&(i.file.as_str(), i.ty.as_str())) {
            hit.insert((i.file.clone(), i.ty.clone()));
            continue;
        }
        violations.push(format!("  {}::{}  =>  {}", i.file, i.ty, nlw));
    }

    assert!(
        violations.is_empty(),
        "\nWALK-STARVATION CONTRACT (rule A, conditional shape) — \
         unreviewed predicate(s):\n{}\n\n\
         A state-dependent needs_legacy_walk() is only sound if it is false \
         exactly when the model is genuinely inert. If it is simply \
         `!uses_scheduler()` (any spelling of the same body) it is accepted \
         automatically; anything else needs a line in CONDITIONAL_ALLOWLIST \
         saying why the false branch covers every reachable state.\n",
        violations.join("\n")
    );

    let stale: Vec<&str> = CONDITIONAL_ALLOWLIST
        .iter()
        .filter(|(f, t, _)| !hit.contains(&(f.to_string(), t.to_string())))
        .map(|(t, _, _)| *t)
        .collect();
    assert!(
        stale.is_empty(),
        "CONDITIONAL_ALLOWLIST has stale entries (delete the lines): {stale:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule B
// ─────────────────────────────────────────────────────────────────────────────

/// A scheduler-driven model's work must have somewhere to come out.
///
/// The walk loop hands back `default()` for every `uses_scheduler()` model, so
/// a model that claims scheduler mode and keeps doing its work in `tick()` —
/// with no `on_event` chain, no `sync_to`, and no matrix level export — is
/// starved even on a bus whose walk is still running. This is the channel that
/// would have made the "fix" for rule A silently useless.
#[test]
fn scheduler_driven_peripherals_declare_a_delivery_path() {
    let impls = parse_peripheral_impls();
    let mut violations = Vec::new();

    for i in &impls {
        let Some(us) = i.body("uses_scheduler") else {
            continue;
        };
        if us == "false" || !i.does_walk_work() {
            continue;
        }
        if DELIVERY_HOOKS.iter().any(|h| i.has(h)) {
            continue;
        }
        violations.push(format!(
            "  {}::{}  uses_scheduler() = {us}  (no {DELIVERY_HOOKS:?})",
            i.file, i.ty
        ));
    }

    assert!(
        violations.is_empty(),
        "\nWALK-STARVATION CONTRACT (rule B) violated by {} model(s):\n{}\n\n\
         The per-cycle walk SKIPS uses_scheduler() models (bus/tick.rs, the \
         `p.dev.uses_scheduler() && !force_scheduler_walk` early return), so \
         their tick() work never runs in production. Declare at least one of \
         {DELIVERY_HOOKS:?} so the work has a route to the CPU.\n",
        violations.len(),
        violations.join("\n")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule C
// ─────────────────────────────────────────────────────────────────────────────

/// True for a literal `legacy_walk_disabled = true` assignment.
///
/// Deliberately narrow: `= self.derive_walk_deletable()`,
/// `= match manifest.walk_deleted { .. }` and every comparison / read are the
/// legitimate forms. Split out so it can be falsified directly — see
/// [`rule_c_detector_is_not_vacuous`], which matters because rule C's only
/// current subject is exempted while its fix is in flight, and an
/// always-passing detector would look identical.
fn is_hand_walk_assert(normalised_line: &str) -> bool {
    normalised_line.contains("legacy_walk_disabled = true")
        && !normalised_line.contains("==")
        && !normalised_line.contains("!=")
}

/// Rule C's detector must fire on the shape it exists to catch.
#[test]
fn rule_c_detector_is_not_vacuous() {
    // The exact classic-ESP32 line (crates/core/src/system/xtensa/esp32.rs).
    assert!(is_hand_walk_assert("bus.legacy_walk_disabled = true;"));
    assert!(is_hand_walk_assert("self.legacy_walk_disabled = true;"));
    // Legitimate forms must NOT fire.
    assert!(!is_hand_walk_assert(
        "bus.legacy_walk_disabled = bus.derive_walk_deletable();"
    ));
    assert!(!is_hand_walk_assert(
        "self.legacy_walk_disabled = self.derive_walk_deletable();"
    ));
    assert!(!is_hand_walk_assert(
        "bus.legacy_walk_disabled = match manifest.walk_deleted {"
    ));
    assert!(!is_hand_walk_assert(
        "if self.legacy_walk_disabled == true {"
    ));
    assert!(!is_hand_walk_assert("legacy_walk_disabled: false,"));
    assert!(!is_hand_walk_assert("return self.legacy_walk_disabled;"));
}

/// Walk deletion is DERIVED from the peripheral set, never asserted by hand.
///
/// `bus.legacy_walk_disabled = true` is a claim about every peripheral on the
/// bus made by code that inspects none of them. On classic ESP32 that claim sat
/// under a comment naming `uart0` as migrated to the scheduler; `Esp32Uart`
/// never was, so its `tx_fifo` was never drained,
/// `UART_STATUS.TXFIFO_CNT` pinned at its high-water mark, and arduino-esp32's
/// wait-for-space loop spun forever. Use `recompute_walk_deletable()` (which
/// asks each peripheral) or the per-config `manifest.walk_deleted` opt-in.
#[test]
fn walk_deletion_is_derived_not_asserted() {
    let root = src_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let pending: BTreeSet<&str> = RULE_C_PENDING_FIX.iter().map(|(f, _)| *f).collect();
    let mut pending_seen: BTreeSet<String> = BTreeSet::new();
    let mut violations = Vec::new();
    let mut scanned_any = false;

    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("tests/") || rel.ends_with("tests_main.rs") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("read source");
        let clean = blank_cfg_test_modules(&blank_comments_and_strings(&raw));
        scanned_any = true;
        for (n, line) in clean.lines().enumerate() {
            let l = normalise(line);
            if !l.contains("legacy_walk_disabled") {
                continue;
            }
            if is_hand_walk_assert(&l) {
                if pending.contains(rel.as_str()) {
                    pending_seen.insert(rel.clone());
                } else {
                    violations.push(format!("  {rel}:{}  {l}", n + 1));
                }
            }
        }
    }

    // A pending entry that no longer fires is not a failure — it is expected to
    // stop firing when the in-flight fix lands, and failing here would make two
    // correct branches conflict. Say so loudly instead.
    for (f, _) in RULE_C_PENDING_FIX {
        if !pending_seen.contains(*f) {
            println!(
                "NOTE: RULE_C_PENDING_FIX entry `{f}` no longer hand-asserts \
                 walk deletion — the in-flight fix has landed. Delete the entry."
            );
        }
    }

    assert!(scanned_any, "rule C scanned no files — the gate is vacuous");
    assert!(
        violations.is_empty(),
        "\nWALK-STARVATION CONTRACT (rule C) — walk deletion asserted by \
         hand:\n{}\n\n\
         Replace with `bus.recompute_walk_deletable()` AFTER the final \
         peripheral is registered, or set `walk_deleted: true` in the system \
         manifest if this is a firmware-specific byte-identity claim no \
         config-time predicate can prove.\n",
        violations.join("\n")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The gate's own falsifiability
// ─────────────────────────────────────────────────────────────────────────────

/// The scanner must actually see a non-default tick and a literal `false`.
///
/// A source-scanning gate fails open the moment its parser drifts (a rename, a
/// formatting change, a macro). This pins the parse on two models whose shapes
/// are known and opposite, so a scanner that silently stops matching turns red
/// here instead of turning every other test in this file vacuously green.
#[test]
fn the_scanner_actually_parses_known_shapes() {
    let impls = parse_peripheral_impls();

    let timer = impls
        .iter()
        .find(|i| i.file == "peripherals/rp2040/timer.rs" && i.ty == "Rp2040Timer")
        .expect("Rp2040Timer impl Peripheral must be found by the scanner");
    assert_eq!(
        timer.body("needs_legacy_walk"),
        Some("!self.scheduler_mode()"),
        "scanner mis-parsed a known conditional predicate"
    );
    assert!(
        timer.does_walk_work(),
        "Rp2040Timer::tick() advances the counter — the scanner must see that"
    );
    assert!(
        timer.has("on_event") && timer.has("take_scheduled_events"),
        "scanner must see the delivery hooks it keys rule B on"
    );

    let stub = impls
        .iter()
        .find(|i| i.file == "peripherals/stub.rs")
        .expect("the stub peripheral must be found");
    assert!(
        !stub.does_walk_work(),
        "the stub peripheral does no walk work; a scanner that thinks it does \
         is over-matching"
    );
}
