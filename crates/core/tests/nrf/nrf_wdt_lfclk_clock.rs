// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every nRF52-family WDT must count CRV in 32.768 kHz LFCLK ticks, not in
//! CPU cycles.
//!
//! The nRF52 PS (§6.34.1) gives the timeout as `(CRV + 1) / 32768` s.
//! `Nrf52Wdt` used to decrement CRV once per CPU cycle, so on a 64 MHz part
//! every watchdog bit about 1953x early: a 1 s Zephyr watchdog fired after
//! about 0.5 ms. The descriptor's `cpu_hz` is the single source of the core
//! clock, and `Peripheral::attach_cpu_hz` is how the bus delivers it.
//!
//! The gate scans every shipped and imported chip descriptor that declares
//! the model, programs CRV = 32 (33 LFCLK ticks), and requires that
//! EVENTS_TIMEOUT stays clear one cycle before `ceil(33 * cpu_hz / 32768)`
//! CPU cycles and is set on that cycle.

use crate::common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

const LFCLK_HZ: u64 = 32_768;

/// Factory spellings of `Nrf52Wdt`.
const WDT_TYPES: &[&str] = &["nrf52840_watchdog", "nrf52_watchdog", "nrf52_wdt"];

const CHIP_DIRS: &[&str] = &["configs/chips", "configs/chips/onboarding"];

const TASKS_START: u64 = 0x000;
const EVENTS_TIMEOUT: u64 = 0x100;
const CRV: u64 = 0x504;

/// CRV for the gate: 33 LFCLK ticks, about 1 ms.
const GATE_CRV: u32 = 32;

fn chip_files_declaring_a_wdt() -> Vec<(PathBuf, ChipDescriptor)> {
    let mut out = Vec::new();
    for dir in CHIP_DIRS {
        let dir = root(dir);
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
            .collect();
        entries.sort();
        for path in entries {
            let chip = ChipDescriptor::from_file(&path)
                .unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
            if chip
                .peripherals
                .iter()
                .any(|p| WDT_TYPES.contains(&p.r#type.as_str()))
            {
                out.push((path, chip));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "no chip descriptor declares an nRF52 WDT; did the model type change?"
    );
    out
}

/// CPU cycles from TASKS_START to EVENTS_TIMEOUT for `crv` at `cpu_hz`.
fn expected_timeout_cycles(crv: u32, cpu_hz: u64) -> u64 {
    (u128::from(crv) + 1)
        .saturating_mul(u128::from(cpu_hz))
        .div_ceil(u128::from(LFCLK_HZ)) as u64
}

#[test]
fn nrf_wdt_times_out_after_crv_plus_one_lfclk_ticks() {
    let mut wrong: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for (path, chip) in chip_files_declaring_a_wdt() {
        let name = path
            .strip_prefix(root("configs/chips"))
            .unwrap_or(&path)
            .display()
            .to_string();
        assert!(
            chip.cpu_hz > 0,
            "{name}: descriptor must declare cpu_hz — the LFCLK ratio depends on it"
        );
        let expected = expected_timeout_cycles(GATE_CRV, chip.cpu_hz);
        for p in &chip.peripherals {
            if !WDT_TYPES.contains(&p.r#type.as_str()) {
                continue;
            }
            // A fresh bus per instance: the WDT is one-shot until reset.
            let manifest = SystemManifest {
                chip: path.to_string_lossy().into_owned(),
                ..Default::default()
            };
            let mut bus = SystemBus::from_config(&chip, &manifest)
                .unwrap_or_else(|e| panic!("assemble {name}: {e}"));
            let base = p.base_address;
            bus.write_u32(base + CRV, GATE_CRV).unwrap();
            bus.write_u32(base + TASKS_START, 1).unwrap();
            for _ in 0..expected - 1 {
                let _ = bus.tick_peripherals_fully_forced();
            }
            let early = bus.read_u32(base + EVENTS_TIMEOUT).unwrap();
            let _ = bus.tick_peripherals_fully_forced();
            let on_time = bus.read_u32(base + EVENTS_TIMEOUT).unwrap();
            if early != 0 || on_time != 1 {
                wrong.push(format!(
                    "{name} `{}` at {base:#x}, cpu_hz={}: EVENTS_TIMEOUT={early} after {} \
                     cycles (want 0), {on_time} after {expected} (want 1)",
                    p.id,
                    chip.cpu_hz,
                    expected - 1
                ));
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "no nRF52 WDT instance was exercised");
    assert!(
        wrong.is_empty(),
        "WDT timeout does not follow (CRV + 1) / 32768 s on {} instance(s):\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

/// The 64 MHz nRF52840 row in absolute numbers: 33 LFCLK ticks is
/// 64_453.125 CPU cycles, so the event lands on cycle 64_454.
#[test]
fn nrf52840_wdt_timeout_cycle_at_64_mhz() {
    assert_eq!(expected_timeout_cycles(GATE_CRV, 64_000_000), 64_454);
}
