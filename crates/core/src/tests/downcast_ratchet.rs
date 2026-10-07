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

use super::source_text::strip_comments_and_strings;
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
/// 193 → 194 / 207 → 208: `SystemBus::observed_of` — ONE generic accessor over
/// the readback-only device registry, which replaced six typed
/// `Vec<Arc<Concrete>>` fields (ws2812 / servos / step_dir_motors /
/// h_bridge_motors / ili9341_parallel / unipolar_steppers) and their six arms
/// in the attached-device walk. This is a deliberate trade and it goes the way
/// the row wants: the debt row 6.5 is about is `as_any()` spread over ~60
/// concrete types, one site per type. What this adds is a single
/// type-parameterised site that serves all six today and every readback-only
/// part added after, so the number stops tracking the number of off-chip parts
/// at all. The alternative was a seventh public field on `SystemBus` the next
/// time somebody adds a stepper.
/// 208 → 210: the SAM SERCOM console joins the by-type and by-name RX-source
/// walks in `bus::construct`. Both walks are a chain of `downcast_ref` arms,
/// one per UART-shaped model (generic `Uart`, `EspUart`, `Nrf52Uarte`,
/// `Nrf54lUarte`), and a new console model that is not in the chain silently
/// gets NO injected serial input — a board that cannot be typed at, with no
/// error to say so. This is the debt row 6.5 names, added knowingly: the fix
/// that would actually retire it is a `UartConsole` capability trait covering
/// `set_sink` and `rx_buffer`, which retires all four existing arms too and is
/// its own change, not a rider on a chip onboarding.
/// 194 → 195: `components::supply::UnpoweredI2cDevice`, the decorator that
/// makes an I²C part with no supply NACK its address. It is the second
/// transparent decorator on the I²C attach chain — `bus_trace::TracingI2cDevice`
/// is the first — and like that one it must forward `as_any()`, or every
/// downcast that reaches an attached slave's concrete type today would start
/// answering `None` for exactly the parts a user is trying to debug. That is
/// evidence disappearing rather than reading dark, which is the failure mode
/// `inspect::DeviceEvidence` exists to end. This is not a new concrete type
/// joining the ~60 the row is about: it is one forward, on a wrapper that has
/// no type of its own to reach for.
///
/// 195 → 196: `components::supply`'s own wiring test. The decorator's other
/// tests build it by hand and so prove nothing about whether anything ever
/// puts it on a device — the "guard not wired to the path that matters" trap.
/// The test that closes it builds a real bus from a real manifest and asks the
/// I²C controller what it would answer at 0x3C, which means reaching
/// `bus.peripherals[..].dev` down to the concrete `peripherals::i2c::I2c`.
/// That is the "a test reaching into a concrete model" case this module's doc
/// names as justified; the alternative is a public accessor that exists solely
/// so one test need not downcast. The same one call site is also the
/// 210 → 211 `downcast_ref`: `as_any()` and `downcast_ref` are the two halves
/// of one reach, and both counters see it.
///
/// 196 → 197 / 211 → 212: Cortex-M wasm-JIT `try_compile_from_bus` needs the
/// concrete `SystemBus` flash/RAM image (same reach RISC-V JIT already uses).
/// The cycle-accurate JIT gate does **not** downcast: it goes through
/// `Bus::requires_cycle_accurate`.
///
/// 197 → 199 / 212 → 214: SAM PORT / RA PORT / i.MX GPIO family dispatch on
/// the maker-five twins reaches the concrete gpio layout through `as_any` /
/// `downcast_ref` (two new sites).
///
/// 199 → 202 / 214 → 217: SPI edge-sampling tests inspect the attached
/// `EdgeSlave`/`EdgeDev` (latched MOSI bytes / call count). Production path
/// does not grow a downcast; these three are test-only.
/// 213 → 210: the two tri-colour e-papers became YAML `display` descriptors,
/// so the CLI's `snapshot` and `test` commands stopped reaching for
/// `Ssd1680Tricolor290` and then `Uc8151dTricolor290` and now take ONE arm on
/// `GenericDisplay`, reading planes by name through `GenericDisplay::planes`.
/// The seven e2e / snapshot / attach tests that reached a panel also collapsed
/// onto that one type. This is the row going the right way for the right
/// reason: the reach that remains is one per PRIMITIVE, not one per part, so
/// the next panel adds none.
///
/// 199 → 200 / 210 → 211: SEGGER RTT host model wiring. `SystemBus` gains
/// `attach_rtt_sink` / `segger_rtt_status` (`bus::construct`); the status walk
/// is one `as_any()` + `downcast_ref` reach for the `SeggerRtt`
/// pseudo-peripheral, which is attached through `add_peripheral` and shares no
/// existing capability with any named console model. The mutable sink attach
/// uses `as_any_mut` / `downcast_mut` and adds no counted site. Retiring the
/// reach means a capability trait over both methods, which is row 6.5's work,
/// not a rider on the RTT feature.
///
/// 200 → 203 / 207 → 210: RTT down-channels and ITM. `bus::construct` reaches
/// three peripherals that share no capability trait yet: Xtensa
/// `RamPeripheral` (an RTT id that lives only in DRAM), `SeggerRtt` (down-
/// channel fill), and `Itm` (stimulus port 0). Mutable writes use
/// `as_any_mut` / `downcast_mut` and add no counted site. A capability trait
/// is row 6.5, not a rider on this feature.
///
/// The scan then started stripping comments and string literals before
/// counting (`super::source_text`). A comment that names `as_any()` stopped
/// counting as a call. Re-derive from a run of this test; do not subtract the
/// old prose delta from the new code delta.
///
/// `maybe_latch_dc` stopped asking "is this an SPI?" with four `TypeId`
/// comparisons. `Peripheral::spi_attached_devices` answers it with a vtable
/// call, so that `as_any()` reach and the four `downcast_ref` attempts
/// (`Spi`, `Esp32Spi`, `Esp32c3Spi`, `Esp32s3Spi`) go. The next SPI kind adds
/// none.
///
/// `dport_cross_core_pending` stopped downcasting `Dport` on the per-
/// instruction IRQ check. `Peripheral::cross_core_pending` is the vtable call.
///
/// The two pad brackets at the MMIO write choke stopped downcasting the
/// written peripheral and stopped scanning for their partner. The indices are
/// resolved once in `rebuild_peripheral_ranges`, one `as_any()` per peripheral,
/// the same shape the FLASH gates already use.
///
/// Ceilings below are the count on this tree after those cuts, including the
/// RTT and ITM reaches above, plus two `as_any()` checks that confirm a cached
/// pad-bracket slot is still that peripheral (`begin_*`). Measured by
/// `the_downcast_count_only_shrinks`.
const MAX_AS_ANY: usize = 198;
// GPIO schedule migration removes four concrete sensor downcasts.
const MAX_DOWNCAST_REF: usize = 197;

/// The MUTABLE half of the same reach, counted from the day it started being
/// counted. Until then the scan matched only `as_any()` and `downcast_ref`,
/// so a `bus.as_any_mut()` + `downcast_mut::<SystemBus>()` reach grew the
/// debt row 6.5 is about while this gate stayed green: the store-spin
/// coalescers (#93) added one on RISC-V and one on Xtensa, both on a per-
/// batch hot path, and nothing saw them. Those two became the
/// `Bus::commit_ram_store_spin` / `Bus::commit_plain_memory_store_spin`
/// capabilities in the change that introduced these ceilings. The RTT sink
/// attach and the ITM stimulus write (notes above) are among the
/// mutable reaches these numbers already contain; they were deliberately
/// uncounted then and are counted now.
///
/// Set at the count on this tree when the counters were added. Same rules as
/// the two above: they may only shrink, and a shrink must lower them.
///
/// Raised 272 -> 274 and 332 -> 338 by the upstream sync to w1ne/main
/// 874e23c8, which carries no ceiling for these two: the i.MX RT eDMA model
/// (`peripherals/imxrt/edma.rs`, +2/+2) and its DMAMUX / bus wiring
/// (`bus/construct.rs` +3 downcast_mut, `bus/attach.rs` +1). Upstream code,
/// not a fork change; moving those reaches onto a capability trait belongs
/// upstream.
///
/// Raised 274 -> 275 and 338 -> 340 by the upstream sync to w1ne/main
/// 647fdbfa, for the same reason: the CAN bridge's single controller reach
/// (`bus/can_bridge_service.rs` `can_ctl`: one `as_any_mut()`, then
/// `downcast_mut` to `Fdcan` or `BxCan`), which upstream wrote as the one
/// downcast site for the whole bridge. Upstream code, not a fork change.
///
/// Raised 340 -> 341 (`downcast_mut` only) when nRF GPIOTE joined
/// `wire_nrf52_pads`: a Task-mode channel owns its pad over the port's
/// DIR/OUT (the micro:bit V2 LED matrix columns), so GPIOTE takes pin-claim
/// tokens exactly as UARTE / TWIM / SPIM already do in the same loop, one
/// `downcast_mut` arm each. The wiring pass is a one-time build step, not a
/// per-cycle reach; moving it and its siblings onto a capability belongs
/// together, upstream.
///
/// Raised 341 -> 342 (`downcast_mut` only) when nRF PWM joined the same loop:
/// a playing `PSEL.OUT[n]` channel owns its pad (duty reaches the circuit),
/// one more `downcast_mut` arm beside GPIOTE's, same one-time wiring step.
///
/// Lowered 275 -> 274: the wasm held-ADC setters (`set_adc_channel` /
/// `clear_adc_channel`) ask the generic `set_adc_channel_input` hooks first
/// and share one STM32 `Adc` fallback, instead of two inline downcasts.
/// `MAX_DOWNCAST_MUT` 342 -> 341 for the same change.
const MAX_AS_ANY_MUT: usize = 274;
const MAX_DOWNCAST_MUT: usize = 341;

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
    as_any_mut: usize,
    downcast_mut: usize,
    files_scanned: usize,
}

fn count() -> Counts {
    let root = repo_root();
    let mut as_any = 0;
    let mut downcast_ref = 0;
    let mut as_any_mut = 0;
    let mut downcast_mut = 0;
    let mut files_scanned = 0;
    for path in rust_sources(&root) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        // This file's own doc comment names both patterns, so it would count
        // itself and drift by its own edits. Stripping (below) blanks the
        // prose, but not the `downcast_ref` field and local this counter keeps
        // — those are real code, and the needle matches substrings, so no
        // rename escapes them. The file stays excluded.
        if path.ends_with("tests/downcast_ratchet.rs") {
            continue;
        }
        // Count code, not prose. The scan used to match the bare word anywhere
        // in a file, so a comment explaining why a downcast was *removed*
        // counted as a downcast: the commit that deleted the per-instruction
        // IPI downcast went red because its own comment said `as_any()`. The
        // gate measured the word rather than the call.
        let code = strip_comments_and_strings(&src);
        files_scanned += 1;
        as_any += code.matches("as_any()").count();
        downcast_ref += code.matches("downcast_ref").count();
        // Needles are disjoint from the two above: `as_any_mut()` does not
        // contain `as_any()`, and `downcast_mut` does not contain
        // `downcast_ref`, so no call is counted twice. The trait DEFINITIONS
        // (`fn as_any_mut(&mut self)`) do not match either, because the
        // needle includes the empty call parentheses.
        as_any_mut += code.matches("as_any_mut()").count();
        downcast_mut += code.matches("downcast_mut").count();
    }
    Counts {
        as_any,
        downcast_ref,
        as_any_mut,
        downcast_mut,
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

    assert!(
        c.as_any_mut <= MAX_AS_ANY_MUT,
        "as_any_mut() call sites rose to {} (ceiling {MAX_AS_ANY_MUT}). The mutable reach is the \
         same debt as as_any(); put the operation on a capability trait (Bus / Peripheral / Cpu \
         method with a default) instead, or raise MAX_AS_ANY_MUT in the same commit that explains \
         why.",
        c.as_any_mut
    );
    assert!(
        c.downcast_mut <= MAX_DOWNCAST_MUT,
        "downcast_mut sites rose to {} (ceiling {MAX_DOWNCAST_MUT}). See the as_any_mut message.",
        c.downcast_mut
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
    assert_eq!(
        c.as_any_mut, MAX_AS_ANY_MUT,
        "as_any_mut() is down to {} but MAX_AS_ANY_MUT is still {MAX_AS_ANY_MUT}. Lower it.",
        c.as_any_mut
    );
    assert_eq!(
        c.downcast_mut, MAX_DOWNCAST_MUT,
        "downcast_mut is down to {} but MAX_DOWNCAST_MUT is still {MAX_DOWNCAST_MUT}. Lower it.",
        c.downcast_mut
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
        c.as_any_mut > 0 && c.downcast_mut > 0,
        "found no mutable downcasts ({} as_any_mut, {} downcast_mut) — the patterns stopped \
         matching, so MAX_AS_ANY_MUT / MAX_DOWNCAST_MUT are meaningless",
        c.as_any_mut,
        c.downcast_mut
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
