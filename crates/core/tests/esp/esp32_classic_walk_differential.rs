// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic walk differential — the browser's scheduling policy must not
//! starve the UART0 model of ticks.
//!
//! ## What this gate is derived from
//!
//! The OBSERVABLE contract, stated without reference to any implementation:
//! *a classic-ESP32 firmware that writes to Serial must have those bytes reach
//! the UART sink when the machine is driven with the browser's own scheduling
//! policy* (`event-scheduler` feature + `max_safe_tick_interval()` +
//! idle fast-forward + batched `AdvanceRequest::run`).
//!
//! The CLI (`labwired test`, `labwired snapshot`) builds `labwired-core`
//! WITHOUT `event-scheduler` (see `crates/cli/Cargo.toml` — the feature is
//! deliberately opt-in there), so every CLI lane in this repo runs the legacy
//! per-cycle walk and cannot observe this class of defect at all. The browser
//! crate (`crates/wasm/Cargo.toml`) pins
//! `labwired-core = { features = ["event-scheduler"] }`. That is the entire
//! delta, and it is why a lab could be 12/12 green on the CLI and livelock in
//! the browser.
//!
//! ## The defect this was written against
//!
//! `configure_xtensa_esp32` hand-set `bus.legacy_walk_disabled = true` with a
//! comment claiming "uart0, gpio, rtc_cntl, timg0/1 migrated to the event
//! scheduler". `Esp32Uart` implemented neither `uses_scheduler()` nor
//! `needs_legacy_walk() == false` AT THE TIME; it drained `tx_fifo` from
//! `tick()` and
//! nowhere else. Under `event-scheduler` the hand flag deleted the walk, so
//! `Esp32Uart::tick()` never ran, `tx_fifo` never drained, `UART_STATUS.
//! TXFIFO_CNT` pinned, and arduino-esp32's `while (128 - txfifo_cnt < 2)`
//! wait-for-space loop spun forever before `loop()` ever ran.
//!
//! ## Why the two arms are gated differently
//!
//! The CONSEQUENCE of the lie needs `event-scheduler` to be observable — with
//! the feature off, `legacy_walk_disabled` is never read and the walk always
//! runs. So the observable arm is feature-gated.
//!
//! The LIE ITSELF is not. `legacy_walk_disabled`, `derive_walk_deletable` and
//! `recompute_walk_deletable` exist in every build; only their *effect* is
//! cfg-gated. So the structural arm compiles and runs under DEFAULT features,
//! which is what lets the fast PR lane carry it at ~zero marginal cost. A gate
//! that only runs nightly is a gate this bug can walk back through.

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::configure_xtensa_esp32;

#[cfg(feature = "event-scheduler")]
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
#[cfg(feature = "event-scheduler")]
use labwired_core::{AdvanceRequest, Bus, Cpu, Machine};
#[cfg(feature = "event-scheduler")]
use std::path::{Path, PathBuf};
#[cfg(feature = "event-scheduler")]
use std::sync::{Arc, Mutex};

/// `UART_STATUS_REG` for UART0 on classic ESP32 (`DR_REG_UART_BASE + 0x1C`).
#[cfg(feature = "event-scheduler")]
const UART0_STATUS: u64 = 0x3FF4_001C;
/// `UART_CLKDIV_REG` for UART0.
#[cfg(feature = "event-scheduler")]
const UART0_CLKDIV: u64 = 0x3FF4_0014;

#[cfg(feature = "event-scheduler")]
fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/core
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// Committed classic-ESP32 fixture (esp-hal + `esp_println` over UART0). This
/// is the same ELF the Tier-1 matrix runs through the CLI, where its serial
/// transcript is parsed and scored — so the firmware demonstrably emits UART
/// bytes on the walk path. Here it is run under the BROWSER's policy instead.
#[cfg(feature = "event-scheduler")]
fn tier1_esp32_elf() -> PathBuf {
    repo_root().join("tests/fixtures/tier1/esp32.elf")
}

/// Build the classic-ESP32 machine exactly the way
/// `WasmSimulator::new_from_config_xtensa_esp32` does: `configure_xtensa_esp32`,
/// a TX sink on UART0, a real second LX6 as APP_CPU, ELF entry + seeded stacks.
#[cfg(feature = "event-scheduler")]
type UartSink = Arc<Mutex<Vec<u8>>>;

#[cfg(feature = "event-scheduler")]
fn browser_like_machine(elf: &Path) -> (Machine<Box<dyn Cpu>>, UartSink) {
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);
    bus.refresh_peripheral_index();

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let app_cpu: Box<dyn Cpu> = Box::new(XtensaLx7::new_app_cpu());
    let mut machine = Machine::new(boxed, bus).with_secondary_cpu(app_cpu);

    let image = labwired_loader::load_elf(elf).expect("parse classic-ESP32 ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine.cpu.set_pc(image.entry_point as u32);
    machine.cpu.set_sp(0x3FFE_0000);
    if let Some(cpu1) = machine.cpu_secondary.as_mut() {
        cpu1.set_sp(0x3FFD_8000);
    }
    (machine, uart_sink)
}

/// The browser's own policy knobs (`apply_browser_c3_policy` in
/// `crates/wasm/src/lib.rs`, and the TS engine init): recommended tick interval
/// straight from the bus, idle fast-forward on.
#[cfg(feature = "event-scheduler")]
fn apply_browser_policy(machine: &mut Machine<Box<dyn Cpu>>) -> u32 {
    let rec = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = rec.max(1);
    machine.bus.config.peripheral_tick_interval = rec.max(1);
    machine.config.idle_fast_forward_enabled = true;
    rec
}

/// STRUCTURAL arm. The bus's `legacy_walk_disabled` flag must be what the
/// conservative config-time derivation actually proves — not a hand-written
/// claim. `derive_walk_deletable` is the only thing that can honestly answer
/// "is every peripheral on this bus walk-independent"; a hand `= true` is an
/// unchecked assertion about models that may since have changed (or never have
/// been migrated at all).
///
/// This arm is cheap, needs no firmware, and would have caught the defect at
/// bus-construction time.
#[test]
fn esp32_classic_walk_flag_is_derived_not_asserted() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let as_built = bus.legacy_walk_disabled;
    bus.recompute_walk_deletable();
    let derived = bus.legacy_walk_disabled;

    if as_built != derived {
        let forcing: Vec<String> = bus
            .peripherals
            .iter()
            .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
            .map(|p| format!("{} @ {:#010x}", p.name, p.base))
            .collect();
        panic!(
            "classic-ESP32 bus asserts legacy_walk_disabled={as_built} but the \
             conservative derivation proves {derived}.\n\
             Peripherals that still FORCE the walk (needs_legacy_walk() && \
             !uses_scheduler()), i.e. the models a hand `= true` would starve \
             of ticks:\n  {}\n\
             Fix the model (migrate it to the scheduler, or prove it inert and \
             override needs_legacy_walk) — do not hand-assert the flag.",
            forcing.join("\n  ")
        );
    }
}

/// OBSERVABLE arm. Serial bytes must reach the sink under the browser's policy.
///
/// Deliberately asserts on the sink (the thing a user sees in the serial pane),
/// not on tick counts or on any flag — so it stays honest no matter how the
/// scheduling is implemented.
#[cfg(feature = "event-scheduler")]
#[test]
fn esp32_classic_serial_reaches_sink_under_browser_policy() {
    let elf = tier1_esp32_elf();
    assert!(
        elf.exists(),
        "committed fixture missing: {elf:?} (tests/fixtures/tier1/esp32.elf)"
    );

    let (mut machine, sink) = browser_like_machine(&elf);
    let tick_interval = apply_browser_policy(&mut machine);

    const BATCH: u64 = 500_000;
    const MAX_STEPS: u64 = 20_000_000;
    let mut steps: u64 = 0;
    while steps < MAX_STEPS {
        if machine
            .advance(AdvanceRequest::run(Some(BATCH)))
            .map(|_| ())
            .is_err()
        {
            break;
        }
        steps += BATCH;
        if !sink.lock().unwrap().is_empty() {
            break;
        }
    }

    let bytes = sink.lock().unwrap().clone();
    let clkdiv = machine.bus.read_u32(UART0_CLKDIV).unwrap_or(0) & 0xF_FFFF;
    let status = machine.bus.read_u32(UART0_STATUS).unwrap_or(0);
    let txfifo_cnt = (status >> 16) & 0xFF;
    let prof = machine.step_profile();

    eprintln!(
        "browser policy: tick_interval={tick_interval} legacy_walk_disabled={} \
         steps={steps} cycles={} idle_ff_skipped={}",
        machine.bus.legacy_walk_disabled,
        machine.total_cycles,
        machine.idle_fast_forward_cycles_skipped
    );
    eprintln!(
        "uart0: CLKDIV={clkdiv} ({clkdiv:#x}) TXFIFO_CNT={txfifo_cnt} sink_bytes={}",
        bytes.len()
    );
    eprintln!(
        "tick profile: peripheral_ticks={} legacy_tick_entries={} ticked_entries={}",
        prof.peripheral_ticks, prof.legacy_tick_entries, prof.peripheral_ticked_entries
    );

    assert!(
        !bytes.is_empty(),
        "classic-ESP32 firmware produced ZERO UART bytes under the browser's \
         scheduling policy after {steps} steps ({} cycles).\n\
         UART0 CLKDIV={clkdiv} ({clkdiv:#x}), UART_STATUS.TXFIFO_CNT={txfifo_cnt}, \
         legacy_walk_disabled={}, tick_interval={tick_interval}, \
         legacy_tick_entries={}.\n\
         A non-zero TXFIFO_CNT with zero sink bytes means the TX FIFO is filling \
         and never draining — Esp32Uart::tick() is not being called.",
        machine.total_cycles,
        machine.bus.legacy_walk_disabled,
        prof.legacy_tick_entries,
    );
}
