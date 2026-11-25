// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every nRF54L GRTC must derive its 1 MHz SYSCOUNTER from the descriptor's
//! `cpu_hz`, not from a compile-time 128 MHz constant.
//!
//! `Nrf54lGrtc` used to convert CPU cycles to SYSCOUNTER ticks with
//! `CPU_HZ_DEFAULT / 1_000_000 == 128` on every part. That is correct only on
//! the 128 MHz application core; it reads 2x slow on the 64 MHz pre-PLL boot
//! state and 8x slow on a 16 MHz core. The descriptor's `cpu_hz` is the single
//! source of the core clock, and `Peripheral::attach_cpu_hz` is how the bus
//! delivers it.
//!
//! The gate assembles the real `nrf54l15` descriptor three times — at its own
//! 128 MHz, with `cpu_hz` rewritten to the 64 MHz pre-PLL boot clock, and at
//! 32 MHz — and requires exactly 1,000 SYSCOUNTER ticks per millisecond in
//! every case. Rewriting only `cpu_hz` keeps the peripheral list, the base
//! address and the bus wiring under test; it does not invent a board.

use crate::common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// GRTC instance type in the nRF54L chip descriptors.
const GRTC_TYPE: &str = "nrf54l_grtc";

/// Descriptor directory scanned for `nrf54l_grtc` instances. A new nRF54L part
/// must be covered without editing this gate.
const CHIP_DIR: &str = "configs/chips";

/// `MODE.SYSCOUNTEREN` — the enable nrfx writes; TASKS_START is the
/// bare-metal path. Either starts the counter.
const MODE: u64 = 0x510;
const MODE_SYSCOUNTEREN: u32 = 1 << 1;
/// `SYSCOUNTER[0].SYSCOUNTERL`.
const SYSCOUNTERL: u64 = 0x720;

/// Every chip descriptor that declares a GRTC, with its parsed contents.
fn chip_files_declaring_a_grtc() -> Vec<(PathBuf, ChipDescriptor)> {
    let dir = root(CHIP_DIR);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    entries.sort();

    let mut out = Vec::new();
    for path in entries {
        let chip = ChipDescriptor::from_file(&path)
            .unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
        if chip.peripherals.iter().any(|p| p.r#type == GRTC_TYPE) {
            out.push((path, chip));
        }
    }
    assert!(
        !out.is_empty(),
        "no chip descriptor declares `{GRTC_TYPE}`; did the model type change?"
    );
    out
}

/// Assemble `chip`, optionally overriding the core clock, start every GRTC,
/// advance one millisecond of simulated time, and return the SYSCOUNTER value
/// each instance reached.
fn syscounter_after_one_millisecond(
    chip_path: &std::path::Path,
    mut chip: ChipDescriptor,
    cpu_hz: Option<u64>,
) -> Vec<(u64, u32)> {
    if let Some(hz) = cpu_hz {
        chip.cpu_hz = hz;
    }
    let bases: Vec<u64> = chip
        .peripherals
        .iter()
        .filter(|p| p.r#type == GRTC_TYPE)
        .map(|p| p.base_address)
        .collect();
    assert!(
        !bases.is_empty(),
        "{}: no `{GRTC_TYPE}` instance; did the model type change?",
        chip_path.display()
    );

    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| {
        panic!(
            "assemble {} at {} Hz: {e}",
            chip_path.display(),
            chip.cpu_hz
        )
    });

    for &base in &bases {
        bus.write_u32(base + MODE, MODE_SYSCOUNTEREN).unwrap();
    }
    let cycles = chip.cpu_hz / 1000;
    // The model has two drive modes. Under `event-scheduler` the GRTC derives
    // its counter from the bus-published cycle clock, so publishing the cycle
    // is the whole advance; the walk is a no-op for this model in that mode
    // (`needs_legacy_walk()` is false and `legacy_advance` returns default),
    // so skipping it only keeps the run cheap and the drive mode unambiguous.
    // Without the feature the walk is the only path.
    bus.set_current_cycle(cycles);
    if !cfg!(feature = "event-scheduler") {
        for _ in 0..cycles {
            let _ = bus.tick_peripherals_fully_forced();
        }
    }
    bases
        .into_iter()
        .map(|base| (base, bus.read_u32(base + SYSCOUNTERL).unwrap()))
        .collect()
}

/// The shipped nRF54L descriptors keep the SYSCOUNTER at 1 MHz.
#[test]
fn every_nrf54l_grtc_counts_one_megahertz_at_the_descriptor_clock() {
    for (path, chip) in chip_files_declaring_a_grtc() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for (base, ticks) in syscounter_after_one_millisecond(&path, chip.clone(), None) {
            assert_eq!(
                ticks, 1000,
                "{name} GRTC at {base:#x}: 1 ms of CPU cycles must be 1000 \
                 SYSCOUNTER ticks"
            );
        }
    }
}

/// The same hardware and bus at the 64 MHz pre-PLL boot clock, and at a 32 MHz
/// ratio row. These are the rows the old fixed 128-cycle tick failed: at
/// 64 MHz it read 500 ticks per millisecond, at 32 MHz 250.
#[test]
fn nrf54l_grtc_follows_the_descriptor_core_clock() {
    let chip_path = root("configs/chips/nrf54l15.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nrf54l15.yaml");
    for hz in [128_000_000u64, 64_000_000, 32_000_000] {
        for (base, ticks) in syscounter_after_one_millisecond(&chip_path, chip.clone(), Some(hz)) {
            assert_eq!(
                ticks, 1000,
                "nrf54l15 GRTC at {base:#x} with cpu_hz={hz}: expected 1000 \
                 SYSCOUNTER ticks per millisecond, got {ticks}"
            );
        }
    }
}
