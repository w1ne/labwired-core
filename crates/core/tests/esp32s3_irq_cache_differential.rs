// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential gate for the walk-free ESP32-S3 interrupt path.
//!
//! # The claim under test
//!
//! `InterruptFabric::per_cycle_aggregation_free` now returns true for an active
//! ESP32-S3 fabric on a walk-deleted bus, which lets `per_cycle_tick_is_trivial`
//! take the fast path and SKIP `refresh_esp32s3_sched_sources` +
//! `recompute_esp32s3_irq_lines` on every guest instruction. That is a fidelity
//! claim, not a performance one: it says the routed state left behind by the
//! MMIO write choke (`sync_esp32s3_irq_write`) and the event path
//! (`deliver_scheduled_irq_levels`) is, at EVERY bus boundary, bit-identical to
//! what re-polling every scheduler-driven peripheral would have produced.
//!
//! # Why it is measured rather than argued
//!
//! Interrupt-delivery latency is the failure mode that does not show up as a
//! wrong pixel. A level that de-asserts one boundary late renders the same
//! frame, right up to the run where the ISR re-enters and the FreeRTOS SMP
//! scheduler wedges (see the `sync_esp32s3_irq_write` doc comment for the
//! self-linking `vListInsert` deadlock this exact latency already caused once).
//! So the gate does not test an outcome downstream of routing — it compares the
//! routing itself, per boundary, cycle by cycle.
//!
//! `SystemBus::install_esp32s3_irq_audit` arms the comparison inside the bus,
//! at the precise fast-path early-return the optimisation added. Every audited
//! boundary computes:
//!
//!   * the CACHED answer — `pending_cpu_irqs` and the intmatrix `INTR_STATUS`
//!     mirror as the choke/event path left them;
//!   * the POLLED answer — a fresh `poll_scheduler_matrix_sources` + recompute,
//!     i.e. exactly what the pre-optimisation per-cycle tick did;
//!
//! and records any disagreement with the cycle it happened on. Both halves of
//! the S3 fabric's output are compared, plus the scheduler-source bitmap they
//! derive from, because a cache that got the slot bitmap right and the
//! `INTR_STATUS` mirror wrong would dispatch the ISR and then hand it the wrong
//! source.
//!
//! # Anti-vacuity
//!
//! A workload whose interrupt path never comes up agrees with a re-poll
//! trivially. Every test therefore asserts on `boundaries` (the audit ran at
//! all — a fast path that was never taken reports zero) and on a liveness
//! witness, not only on `divergences.is_empty()`. Where the boundary grid is
//! fine enough the witness is `boundaries_with_routed_irq`, an interrupt
//! pending at the cores when the audit looked; at interval 8 it is the ISR
//! ENTRY COUNT, because there the assert window fits between two boundaries.
//! See `assert_routed_irq_was_observed`.
//!
//! # Negative control
//!
//! Delete the intmatrix arm of `sync_esp32s3_irq_write` in
//! `crates/core/src/bus/routing.rs`, i.e. change
//!
//! ```text
//! let relevant = Some(idx) == self.irq_fabric.esp32s3.intmatrix_idx
//!     || self.peripherals.get(idx).is_some_and(|p| p.dev.uses_scheduler());
//! ```
//!
//! to drop the `intmatrix_idx` term, and a MAP-register write stops re-routing
//! at the write. `alarm_isr_routing_is_identical_to_a_full_repoll` then fails
//! with a divergence at the boundary after the binding is programmed. Dropping
//! the `uses_scheduler()` term instead breaks INT_CLR de-assert and fails both
//! tests. Verified red both ways; a gate that cannot go red proves nothing.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::{Esp32s3IrqAudit, SystemBus};
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::peripherals::esp32s3::systimer::Systimer;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
use labwired_core::{AdvanceRequest, BreakpointPolicy, Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct CountingObserver(AtomicU64);
impl CountingObserver {
    fn isr_entries(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl labwired_core::peripherals::esp32s3::gpio::GpioObserver for CountingObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, _c: u64) {
        if pin == 2 && !from && to {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Render an audit result for a failure message: the first divergences in full,
/// so the cycle and both answers are on screen without a re-run.
fn report(audit: &Esp32s3IrqAudit) -> String {
    let mut out = format!(
        "boundaries={} with_routed_irq={} with_sched_sources={} \
         sched_source_union={:016x?} divergences={}",
        audit.boundaries,
        audit.boundaries_with_routed_irq,
        audit.boundaries_with_sched_sources,
        audit.sched_source_union,
        audit.divergence_count
    );
    for d in &audit.divergences {
        out.push_str(&format!(
            "\n  cycle {:>12}: routed cached={:08x?} polled={:08x?} \
             | INTR_STATUS cached={:08x?} polled={:08x?} \
             | sched_sources cached={:016x?} polled={:016x?}",
            d.cycle,
            d.cached_routed,
            d.polled_routed,
            d.cached_intr_status,
            d.polled_intr_status,
            d.cached_sched_sources,
            d.polled_sched_sources,
        ));
    }
    out
}

/// The audit actually ran. Zero boundaries means the walk-free fast path was
/// never taken, so nothing about it was proved — the shape of green that a
/// fully-skipped gate has.
fn assert_audit_ran<'a>(bus: &'a SystemBus, what: &str) -> &'a Esp32s3IrqAudit {
    let audit = bus
        .esp32s3_irq_audit()
        .unwrap_or_else(|| panic!("{what}: the audit was never installed"));
    assert!(
        audit.boundaries > 0,
        "{what}: ZERO walk-free boundaries audited. The S3 never took the \
         walk-free fast path, so this gate proved nothing about it — check \
         `legacy_walk_disabled` and `per_cycle_tick_is_trivial`. {}",
        report(audit)
    );
    audit
}

/// The cached and polled answers never disagreed. This is the gate.
fn assert_no_divergence(audit: &Esp32s3IrqAudit, what: &str) {
    assert!(
        audit.divergences.is_empty(),
        "{what}: the walk-free S3 interrupt path disagreed with a full re-poll. \
         The write choke (`sync_esp32s3_irq_write`), the event path \
         (`deliver_scheduled_irq_levels`) or the dispatch re-derivation \
         (`resettle_cpu_irq_levels`) missed a level change, so an interrupt is \
         delivered — or read back — at the wrong cycle. {}",
        report(audit)
    );
}

/// An interrupt was actually pending AT AN AUDITED BOUNDARY, so the comparison
/// had something to compare.
///
/// Only usable where the boundary grid is fine enough to catch the assert
/// window: at `peripheral_tick_interval` 1 (which is what every DUAL-core S3
/// machine is pinned to — see `a_dual_core_machine_never_relaxes_the_peripheral_tick_interval`)
/// the audit samples every cycle and sees the level. At a wider interval the
/// event path can deliver, the core dispatch, and the ISR acknowledge entirely
/// between two boundaries; the interval-8 test below uses ISR-entry COUNT as
/// its liveness witness instead of pretending this one applies.
fn assert_routed_irq_was_observed(audit: &Esp32s3IrqAudit, what: &str) {
    assert!(
        audit.boundaries_with_routed_irq > 0,
        "{what}: the routed per-core slot bitmap was zero at every one of the \
         {} audited boundaries — no interrupt was ever pending when the audit \
         looked, so agreeing with a re-poll is vacuous. {}",
        audit.boundaries,
        report(audit)
    );
}

// ── Workload 1: hand-built periodic-alarm ISR chain ──────────────────────────
//
// The same machine `esp32s3_walk_differential` uses: SYSTIMER ALARM0 in period
// mode → intmatrix source 57 → CPU slot 12 → a hand-assembled ISR that toggles
// GPIO2 and acknowledges with SYSTIMER_INT_CLR. Small, fully deterministic, and
// it exercises the two things the choke has to get right — a level ARMED by the
// scheduler and a level CLEARED by an MMIO write — on every ISR entry.

/// Hand-assembled ISR: GPIO2 0→1 (W1TS), GPIO2 1→0 (W1TC), SYSTIMER_INT_CLR
/// ack, `rfe`.
const ISR_BYTES: &[u8] = &[
    0x69, 0x03, // s32i.n  a6, a3, 0   (GPIO_OUT_W1TS: pin 2 0->1)
    0x69, 0x04, // s32i.n  a6, a4, 0   (GPIO_OUT_W1TC: pin 2 1->0)
    0x79, 0x05, // s32i.n  a7, a5, 0   (SYSTIMER_INT_CLR: ack alarm 0)
    0x00, 0x30, 0x00, // rfe
];

/// `j 0` — jump-to-self spin loop, 3 bytes.
const SPIN_BYTES: &[u8] = &[0x06, 0xff, 0xff];

const IRAM_BASE: u32 = 0x4037_0000;
const ISR_PC: u32 = IRAM_BASE + 0x1000;
const VECBASE_VALUE: u32 = ISR_PC - 0x300;
const SYSTIMER_BASE: u32 = 0x6002_3000;
const INTMATRIX_BASE: u32 = 0x600C_2000;
const SYSTIMER_TARGET0_SOURCE: u32 = 57;
const CPU_IRQ_SLOT: u8 = 12;

/// Guest instructions each alarm workload retires.
const RUN_STEPS: u64 = 400_000;
/// ISR entries the chain delivers over [`RUN_STEPS`]: the alarm period is 20
/// SYSTIMER ticks at 16 MHz against a 240 MHz core = 300 CPU cycles, so
/// 400_000 / 300 = 1333. Pinned so a change in what the audit covered shows up
/// as a changed workload rather than as a quietly smaller gate.
const EXPECTED_ISR_ENTRIES: u64 = 1333;

fn build_alarm_machine(tick_interval: u32) -> (Machine<XtensaLx7>, Arc<CountingObserver>) {
    build_alarm_machine_with_intenable(tick_interval, true)
}

/// `intenable = false` leaves the CPU masked so the alarm level LATCHES instead
/// of being dispatched and acknowledged — the state a mid-run re-bind needs.
fn build_alarm_machine_with_intenable(
    tick_interval: u32,
    intenable: bool,
) -> (Machine<XtensaLx7>, Arc<CountingObserver>) {
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let obs = Arc::new(CountingObserver::default());
    wiring.add_gpio_observer(&mut bus, obs.clone());
    let mut cpu = wiring.cpu;

    for (i, &b) in SPIN_BYTES.iter().enumerate() {
        bus.write_u8((IRAM_BASE + i as u32) as u64, b).unwrap();
    }
    for (i, &b) in ISR_BYTES.iter().enumerate() {
        bus.write_u8((ISR_PC + i as u32) as u64, b).unwrap();
    }

    use labwired_core::cpu::xtensa_sr::{INTENABLE, VECBASE};
    cpu.sr.write(VECBASE, VECBASE_VALUE);
    cpu.regs.write_logical(3, 0x6000_4008); // GPIO_OUT_W1TS
    cpu.regs.write_logical(4, 0x6000_400C); // GPIO_OUT_W1TC
    cpu.regs.write_logical(5, SYSTIMER_BASE + 0x6C); // SYSTIMER_INT_CLR
    cpu.regs.write_logical(6, 0x0000_0004); // bit 2 mask
    cpu.regs.write_logical(7, 0x0000_0001); // alarm 0 clear bit

    // Program the intmatrix binding. Under the walk-free path this MAP write is
    // the ONLY thing that makes source 57 route to slot 12 — nothing re-derives
    // it per cycle any more — so it is also the first thing the negative control
    // breaks.
    bus.write_u32(
        (INTMATRIX_BASE + SYSTIMER_TARGET0_SOURCE * 4) as u64,
        CPU_IRQ_SLOT as u32,
    )
    .unwrap();

    // SYSTIMER ALARM0: PERIOD mode, period 20 SYSTIMER ticks.
    bus.write_u32((SYSTIMER_BASE + 0x1C) as u64, 0).unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x20) as u64, 20).unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x34) as u64, (1u32 << 30) | 20)
        .unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x50) as u64, 1).unwrap();
    let conf = bus.read_u32(SYSTIMER_BASE as u64).unwrap();
    bus.write_u32(SYSTIMER_BASE as u64, conf | (1u32 << 24))
        .unwrap();
    bus.write_u32((SYSTIMER_BASE + 0x64) as u64, 1).unwrap(); // INT_ENA bit 0

    cpu.sr
        .write(INTENABLE, if intenable { 1u32 << CPU_IRQ_SLOT } else { 0 });
    cpu.ps.set_intlevel(0);
    cpu.ps.set_excm(false);
    cpu.set_pc(IRAM_BASE);

    // The SYSTIMER must be scheduler-driven for this gate to mean anything: a
    // legacy-walk SYSTIMER would keep the walk on and the fast path off.
    assert!(
        bus.peripherals.iter().any(|p| p
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<Systimer>())
            .is_some()),
        "the S3 bus must register a Systimer"
    );

    bus.install_esp32s3_irq_audit();

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    (machine, obs)
}

fn run_steps(machine: &mut Machine<XtensaLx7>, steps: u64) {
    const CHUNK: u32 = 100_000;
    let mut done = 0u64;
    while done < steps {
        let n = CHUNK.min((steps - done) as u32);
        machine.run(Some(n)).expect("run S3 alarm machine");
        done += n as u64;
    }
}

/// The always-on half of the gate: fast, hermetic, no fixture, and it runs in
/// every lane that builds `--features event-scheduler`.
///
/// Interval 1 is the load-bearing case. It is what every dual-core ESP32-S3
/// machine runs at, and it is the only interval at which the audit samples
/// every cycle — so a level that is asserted, dispatched and acknowledged
/// inside one guest instruction still cannot hide from it.
#[test]
fn alarm_isr_routing_is_identical_to_a_full_repoll() {
    let (mut machine, obs) = build_alarm_machine(1);
    assert!(
        machine.bus.legacy_walk_disabled,
        "the S3 alarm bus must derive walk deletion; without it the fast path \
         this gate audits is never taken"
    );
    run_steps(&mut machine, RUN_STEPS);
    let entries = obs.isr_entries();
    assert_eq!(
        entries, EXPECTED_ISR_ENTRIES,
        "the alarm chain delivered {entries} ISRs, expected {EXPECTED_ISR_ENTRIES}. \
         The workload changed, so what the audit covered changed with it."
    );
    let audit = assert_audit_ran(&machine.bus, "S3 alarm chain @ interval 1");
    assert_routed_irq_was_observed(audit, "S3 alarm chain @ interval 1");
    assert_no_divergence(audit, "S3 alarm chain @ interval 1");
}

/// The same chain at the interval a batching bus raises to.
///
/// Two distinct things are checked here, and the liveness witness is NOT the
/// one interval 1 uses. At interval 8 the boundary grid is coarser than the
/// window in which the alarm asserts: the event path delivers at the exact
/// cycle, the core dispatches at the next instruction and the ISR's INT_CLR
/// acknowledges four instructions later, all typically between two boundaries.
/// So `boundaries_with_routed_irq` is legitimately zero and asserting on it
/// would be asserting on the sampler, not on the model. What this test does
/// assert is that delivery is UNCHANGED (identical ISR count to interval 1 —
/// a level-latched alarm can never be missed) and that no boundary ever caught
/// the walk-free path holding state a re-poll disagreed with, which is what a
/// LATCHED stale level would look like.
#[test]
fn alarm_isr_routing_is_identical_to_a_full_repoll_at_interval_8() {
    let (mut machine, obs) = build_alarm_machine(8);
    run_steps(&mut machine, RUN_STEPS);
    let entries = obs.isr_entries();
    assert_eq!(
        entries, EXPECTED_ISR_ENTRIES,
        "interval 8 delivered {entries} alarm ISRs, interval 1 delivers \
         {EXPECTED_ISR_ENTRIES}. A level-latched alarm cannot be missed, so a \
         different count is a delivery defect, not quantisation."
    );
    let audit = assert_audit_ran(&machine.bus, "S3 alarm chain @ interval 8");
    assert_no_divergence(audit, "S3 alarm chain @ interval 8");
}

/// Re-binding a matrix source WHILE its level is asserted.
///
/// The write choke has two arms — a scheduler-driven peripheral's registers,
/// and the interrupt matrix itself — and the second one is only load-bearing in
/// this situation: firmware rewrites a `MAP` register while the source behind
/// it is already asserting, so the routed slot has to move with no source-level
/// change and no scheduler event to trigger a re-derivation. esp-hal does
/// exactly this when it binds a handler to an already-pending peripheral.
///
/// The alarm chain above cannot reach it: there the MAP is programmed before
/// anything asserts, so dropping the intmatrix arm of `sync_esp32s3_irq_write`
/// leaves those tests GREEN. Verified — which is why this test exists rather
/// than the coverage being assumed.
///
/// The ISR is deliberately never dispatched (INTENABLE left clear), so the
/// SYSTIMER alarm latches asserted and the re-bind lands on a live level.
#[test]
fn rebinding_a_matrix_source_while_it_asserts_is_re_routed_at_the_write() {
    const OTHER_SLOT: u32 = 17;
    let (mut machine, obs) = build_alarm_machine_with_intenable(1, false);

    // Long enough for the alarm to fire and latch: nothing acknowledges it.
    run_steps(&mut machine, 5_000);
    assert_eq!(
        obs.isr_entries(),
        0,
        "the ISR must never run here — this test needs the level LATCHED, and a \
         dispatched ISR would acknowledge it"
    );
    let before = machine.bus.pending_cpu_irqs[0];
    assert_ne!(
        before, 0,
        "the SYSTIMER alarm must be routed and latched before the re-bind, or \
         the re-bind lands on nothing and this test proves nothing"
    );

    // Re-bind source 57 to a different CPU slot with the level still up. Only
    // the intmatrix arm of the write choke can move the routed bitmap here.
    machine
        .bus
        .write_u32(
            (INTMATRIX_BASE + SYSTIMER_TARGET0_SOURCE * 4) as u64,
            OTHER_SLOT,
        )
        .unwrap();
    assert_eq!(
        machine.bus.pending_cpu_irqs[0],
        1u32 << OTHER_SLOT,
        "the re-bind must move the routed slot AT THE WRITE. Routed is {:#x}, \
         expected {:#x} — the intmatrix arm of `sync_esp32s3_irq_write` is not \
         re-deriving.",
        machine.bus.pending_cpu_irqs[0],
        1u32 << OTHER_SLOT
    );

    run_steps(&mut machine, 5_000);
    let audit = assert_audit_ran(&machine.bus, "S3 matrix re-bind under a live level");
    assert_routed_irq_was_observed(audit, "S3 matrix re-bind under a live level");
    assert_no_divergence(audit, "S3 matrix re-bind under a live level");
}

// ── Workload 2: real ROM-boot firmware ───────────────────────────────────────

fn tier1_flash_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tier1/esp32s3-flash.bin")
}

/// ROM-boot an ESP32-S3 flash image with the audit armed and return the
/// finished machine plus the console it printed to.
///
/// Mirrors `e2e_esp32s3_flash_boot_no_elf`: real mask ROM, real 2nd-stage
/// bootloader, flash MMU, both cores, and `advance` with a wide batch cap —
/// the path the hosted CLI and the browser both issue, and the only one that
/// plans multi-instruction windows. Single-stepping would hide every batching
/// defect, which is the family this gate is aimed at.
fn rom_boot_audited(flash: Vec<u8>, steps: u64) -> (Machine<XtensaLx7>, String) {
    let mut bus = SystemBus::new();
    let flash_size = (flash.len() as u32)
        .next_power_of_two()
        .max(4 * 1024 * 1024);
    let opts = Esp32s3Opts {
        real_reset_boot: true,
        flash_image: Some(flash),
        flash_size,
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    assert_eq!(
        wiring.boot_mode,
        Esp32s3BootMode::Faithful,
        "this gate audits the ROM-boot path the browser runs; without the real \
         ROM there is nothing to boot"
    );
    let mut cpu = wiring.cpu;
    cpu.faithful_windows = true;

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    bus.attach_usb_serial_jtag_sink(sink.clone());
    bus.refresh_peripheral_index();
    assert!(
        bus.legacy_walk_disabled,
        "the ROM-boot S3 bus must derive walk deletion; without it the fast \
         path this gate audits is never taken"
    );
    bus.install_esp32s3_irq_audit();

    let mut app_cpu = XtensaLx7::new_app_cpu();
    app_cpu.faithful_windows = true;
    let mut machine = Machine::new(cpu, bus).with_secondary_cpu(app_cpu);

    let mut retired = 0u64;
    while retired < steps {
        let request = AdvanceRequest::run(Some(steps - retired))
            .with_batch_cap(std::num::NonZeroU32::new(10_000).expect("non-zero"))
            .with_breakpoints(BreakpointPolicy::Ignore);
        let Ok(report) = machine.advance(request) else {
            break;
        };
        if report.primary_steps == 0 {
            break;
        }
        retired += report.primary_steps;
    }
    let out = String::from_utf8_lossy(&sink.lock().unwrap().clone()).to_string();
    (machine, out)
}

/// The committed ROM-boot lane: real mask ROM, real bootloader, real
/// application, ~20 M guest instructions, every one of them a walk-free
/// boundary the audit checks.
///
/// **What this covers and what it does not.** Measured over the tier-1 image:
/// 19,997,999 audited boundaries and ZERO of them with any scheduler-driven
/// matrix source asserting. That firmware polls; it never raises an interrupt
/// through the intmatrix at all. So this test is NOT the interrupt-traffic
/// witness — `assert_routed_irq_was_observed` would be asserting on a workload
/// that cannot satisfy it, and quietly deleting the assertion would leave a
/// gate that looks stronger than it is. What it DOES prove is the other half:
/// that a real ROM boot takes the walk-free fast path 20 M times, and that
/// across all of them the routed state the choke leaves behind never drifts
/// from a full re-poll — which is exactly what a latched stale level would
/// look like. Interrupt traffic is covered by the alarm chain above, by
/// `esp32s3_smp_yield_latency` for the cross-core doorbell, and by the Doom
/// lane below.
///
/// Deliberately NOT behind `esp32s3-fixtures`: that feature exists for tests
/// that BUILD firmware with the +esp toolchain, it is in no CI feature set, and
/// putting a fidelity gate behind it would make it a gate that never runs. The
/// image is a committed repository fixture, so this asserts it is present
/// rather than skipping — a skipped gate reads exactly like a passing one.
///
/// `#[ignore]` in debug only: seconds in release, minutes in a debug build. The
/// release lane (`cargo test --release -p labwired-core --features
/// event-scheduler`) runs it.
#[cfg_attr(
    debug_assertions,
    ignore = "20M-step ROM boot; runs in the release event-scheduler lane"
)]
#[test]
fn rom_boot_firmware_never_latches_a_stale_routed_level() {
    let path = tier1_flash_path();
    let flash = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "the walk-free S3 interrupt gate needs the committed tier-1 flash \
             image at {} ({e}). It is tracked in this repository; refusing to \
             skip, because a skipped fidelity gate reads as a passing one.",
            path.display()
        )
    });
    let (machine, out) = rom_boot_audited(flash, 20_000_000);
    assert!(
        out.contains("TIER1 done"),
        "the firmware never ran to completion, so the audit covered a boot that \
         did not happen. Got: {out:?}"
    );
    let audit = assert_audit_ran(&machine.bus, "tier-1 ESP32-S3 ROM boot");
    assert!(
        audit.boundaries > 19_000_000,
        "expected ~20 M walk-free boundaries over the tier-1 boot, got {}. \
         Something is keeping the S3 off the fast path for most of the run. {}",
        audit.boundaries,
        report(audit)
    );
    assert_no_divergence(audit, "tier-1 ESP32-S3 ROM boot");
}

/// The interrupt-heavy real workload: the ESP32-S3 Doom lab, ROM-booted through
/// the real mask ROM into SMP ESP-IDF.
///
/// This is the run the walk-free path was built for — a FreeRTOS tick ISR at
/// the scheduler rate, `FROM_CPU` yield doorbells, UART and USB-Serial-JTAG
/// levels, all interleaved with the coalesced idle batches that made the write
/// choke necessary. Unlike the tier-1 image it raises real interrupts, so this
/// lane carries the routed-IRQ witness.
///
/// `#[ignore]`d because its firmware input is an 8.5 MB flash image that lives
/// in the MONOREPO's `packages/playground/public/wasm/`, not in this
/// repository. Run it with the image in hand:
///
/// ```text
/// LABWIRED_ESP32S3_DOOM_FLASH=<...>/demo-esp32s3-doom-lab-flash.bin \
///   cargo test --release -p labwired-core --features event-scheduler \
///   --test esp32s3_irq_cache_differential -- --ignored --nocapture doom
/// ```
///
/// It asserts rather than skips when the variable is set but the file is not
/// readable, and skips only when the variable is absent — the one case where
/// the input genuinely is not available in this checkout.
#[test]
#[ignore = "needs the monorepo Doom flash image via LABWIRED_ESP32S3_DOOM_FLASH; run with --release --ignored"]
fn doom_lab_rom_boot_routing_is_identical_to_a_full_repoll() {
    const ENV: &str = "LABWIRED_ESP32S3_DOOM_FLASH";
    let Some(path) = std::env::var_os(ENV).map(PathBuf::from) else {
        eprintln!("{ENV} not set — the Doom lane needs the monorepo flash image");
        return;
    };
    let flash = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{ENV} points at {} which cannot be read: {e}",
            path.display()
        )
    });
    let (machine, out) = rom_boot_audited(flash, 40_000_000);
    assert!(
        out.contains("cpu_start") || out.contains("app_main") || out.contains("doom"),
        "the Doom image never reached ESP-IDF bring-up, so the audit covered a \
         boot that did not happen. Got the first 2 kB: {:?}",
        &out[..out.len().min(2048)]
    );
    let audit = assert_audit_ran(&machine.bus, "ESP32-S3 Doom lab ROM boot");
    eprintln!("S3_IRQ_AUDIT doom: {}", report(audit));
    assert_routed_irq_was_observed(audit, "ESP32-S3 Doom lab ROM boot");
    assert_no_divergence(audit, "ESP32-S3 Doom lab ROM boot");
}
