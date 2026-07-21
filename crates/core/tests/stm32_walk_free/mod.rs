// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared harness for the per-chip STM32 **walk-vs-scheduler** executing
//! differentials (`stm32f401_walk_differential`, `stm32l073_walk_differential`,
//! `stm32l476_walk_differential`, `stm32h563_walk_differential`).
//!
//! Each chip test loads a REAL committed firmware fixture (the stock Zephyr
//! `hello_world`, whose kernel tick is driven from the Cortex **SysTick** and
//! whose console output is deterministic) onto that chip's `from_config` bus and
//! runs it twice:
//!
//!   * the **reference** lane pins the scheduler-migrated core timing blocks
//!     (SysTick + SCB + DWT) and the SoC `Timer`/`Uart`/`Adc`/`Dma1` models back
//!     onto the legacy per-cycle walk (`force_legacy_walk`), and
//!   * the **candidate** lane leaves them scheduler-driven (the production path
//!     under the `event-scheduler` feature).
//!
//! A snapshot vector — `(total_cycles, pc, uart-stream length, SRAM hash)` taken
//! every `snapshot_every` instructions plus the final UART byte stream — must be
//! **byte-identical** between the two lanes. Because the Zephyr kernel schedules
//! off the SysTick IRQ cadence, any drift in the scheduler's tick/IRQ delivery
//! timing (relative to the walk reference) perturbs the kernel's uptime, the
//! console banner timing and the resulting SRAM, so equality is a genuine
//! IRQ-cadence + timing gate on that chip's real boot, not a static register
//! check.
//!
//! Divergence here is a real fidelity bug in the scheduler migration for that
//! chip — it must be reported, never masked.

#![allow(dead_code)]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::adc::Adc;
use labwired_core::peripherals::dma::Dma1;
use labwired_core::peripherals::dwt::Dwt;
use labwired_core::peripherals::scb::Scb;
use labwired_core::peripherals::systick::Systick;
use labwired_core::peripherals::timer::Timer;
use labwired_core::peripherals::uart::Uart;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// SRAM window hashed into each snapshot (16 KiB at the common 0x2000_0000
/// base — mapped on every STM32 family covered here).
const SRAM_BASE: u64 = 0x2000_0000;
const SRAM_HASH_LEN: u64 = 0x4000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub cycles: u64,
    pub pc: u32,
    pub uart_len: usize,
    pub sram_hash: u64,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn load_system(chip_name: &str, system_name: &str) -> (ChipDescriptor, SystemManifest) {
    let chip_path = workspace_root()
        .join("configs/chips")
        .join(format!("{chip_name}.yaml"));
    let sys_path = workspace_root()
        .join("configs/systems")
        .join(format!("{system_name}.yaml"));
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_name}: {e}"));
    let mut manifest = SystemManifest::from_file(&sys_path)
        .unwrap_or_else(|e| panic!("load system {system_name}: {e}"));
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    (chip, manifest)
}

/// Pin every scheduler-migrated model on the bus back onto the legacy per-cycle
/// walk, forming the reference lane. Peripheral types not present on a given
/// chip are simply skipped; any model left scheduler-driven here is
/// scheduler-driven in BOTH lanes, so it contributes identically.
fn force_all_legacy_walk(bus: &mut SystemBus) {
    for entry in bus.peripherals.iter_mut() {
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        if let Some(p) = any.downcast_mut::<Systick>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Scb>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Dwt>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Timer>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Uart>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Adc>() {
            p.force_legacy_walk();
        } else if let Some(p) = any.downcast_mut::<Dma1>() {
            p.force_legacy_walk();
        }
    }
}

fn sram_hash(bus: &SystemBus) -> u64 {
    // FNV-1a over the SRAM window (word reads; unmapped words fault → treated as
    // a stable sentinel so both lanes agree on any hole).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut addr = SRAM_BASE;
    while addr < SRAM_BASE + SRAM_HASH_LEN {
        let word = bus.read_u32(addr).unwrap_or(0xDEAD_BEEF);
        hash ^= word as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        addr += 4;
    }
    hash
}

/// Run one lane to completion, snapshotting every `snapshot_every` instructions.
fn run_lane(
    chip_name: &str,
    system_name: &str,
    fixture: &str,
    reference: bool,
    cycles: u64,
    snapshot_every: u64,
) -> (Vec<Snapshot>, Vec<u8>) {
    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("build {chip_name} bus: {e}"));

    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    if reference {
        force_all_legacy_walk(&mut bus);
        // Pinning a peripheral back onto the walk after `from_config` derived a
        // walk-DELETED bus would starve it of per-cycle ticks; recompute the
        // flag over the live set so the walk reference actually runs.
        bus.recompute_walk_deletable();
    }
    let mut machine = Machine::new(cpu, bus);
    // Pin the scheduler drain to one instruction per tick (the walk reference
    // ticks every cycle unconditionally), so lazy free-running-counter reads are
    // NOT quantised to a coarse batch grid — the exact-match regime the
    // per-model walk differentials are calibrated to.
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;

    let fixture_path = workspace_root().join("tests/fixtures").join(fixture);
    let image = labwired_loader::load_elf(&fixture_path)
        .unwrap_or_else(|e| panic!("load ELF {fixture_path:?}: {e}"));
    machine
        .load_firmware(&image)
        .expect("load firmware into machine");

    let mut snaps = Vec::new();
    let mut done = 0u64;
    while done < cycles {
        let chunk = snapshot_every.min(cycles - done);
        machine.run(Some(chunk as u32)).expect("run chunk");
        done += chunk;
        snaps.push(Snapshot {
            cycles: machine.total_cycles,
            pc: machine.cpu.get_pc(),
            uart_len: uart_sink.lock().unwrap().len(),
            sram_hash: sram_hash(&machine.bus),
        });
    }
    let uart = uart_sink.lock().unwrap().clone();
    (snaps, uart)
}

/// The shared gate body: build the walk reference and the scheduler candidate,
/// assert the snapshot trace and UART stream are byte-identical, and assert the
/// firmware genuinely executed (the deterministic banner appeared).
pub fn assert_walk_free_boot_identical(
    chip_name: &str,
    system_name: &str,
    fixture: &str,
    expected_uart: &[u8],
    cycles: u64,
    snapshot_every: u64,
) {
    let (walk_snaps, walk_uart) = run_lane(
        chip_name,
        system_name,
        fixture,
        true,
        cycles,
        snapshot_every,
    );
    let (sched_snaps, sched_uart) = run_lane(
        chip_name,
        system_name,
        fixture,
        false,
        cycles,
        snapshot_every,
    );

    assert_eq!(
        walk_snaps.len(),
        sched_snaps.len(),
        "{chip_name}: snapshot count mismatch"
    );
    for (i, (w, s)) in walk_snaps.iter().zip(sched_snaps.iter()).enumerate() {
        assert_eq!(
            w,
            s,
            "{chip_name}: walk-vs-scheduler diverged at snapshot {i} \
             (window ~{} instructions)\n  walk={w:?}\n  sched={s:?}",
            (i as u64 + 1) * snapshot_every
        );
    }

    assert_eq!(
        walk_uart, sched_uart,
        "{chip_name}: UART stream diverged between walk and scheduler lanes"
    );

    // The firmware must have actually run its application logic (not spun in a
    // reset loop) — otherwise the byte-identity above would be vacuous.
    assert!(
        walk_uart
            .windows(expected_uart.len())
            .any(|w| w == expected_uart),
        "{chip_name}: firmware did not emit the expected banner {:?}; got {:?}",
        String::from_utf8_lossy(expected_uart),
        String::from_utf8_lossy(&walk_uart),
    );

    // And total_cycles must have advanced identically to the full budget.
    let last = walk_snaps.last().expect("at least one snapshot");
    assert_eq!(
        last.cycles, cycles,
        "{chip_name}: total_cycles did not reach the instruction budget"
    );
}
