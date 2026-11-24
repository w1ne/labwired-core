// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every nRF RTC counts LFCLK (32.768 kHz), not CPU cycles.
//!
//! `Nrf52Rtc` advances one LFCLK edge per `LFCLK_ACCUM_INC` CPU cycles. That
//! ratio depends on the core clock, and the same model runs on three very
//! different cores:
//!
//! | part                  | core     | counter if the core clock is ignored |
//! |-----------------------|----------|--------------------------------------|
//! | nRF51822              | 16 MHz   | 4x slow                              |
//! | nRF52832/833/840      | 64 MHz   | correct (that ratio is the default)   |
//! | nRF5340               | 128 MHz  | 2x fast                              |
//!
//! A wrong RTC rate is silent: a console still prints and a compare still
//! fires, just at the wrong instant, so every `k_msleep` / `millis()` drifts.
//! The bus passes `ChipDescriptor::cpu_hz` through `Peripheral::attach_cpu_hz`;
//! this gate proves the model uses it, for every descriptor that declares an
//! RTC.
//!
//! Sources: nRF51822 PS v3.1 §10 (RTC, 32.768 kHz LFCLK), nRF52840 PS v1.7
//! §6.21, nRF5340 PS v1.5 §6.21. LFCLK is the same 32.768 kHz crystal on all
//! three families.

use crate::common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// LFCLK nominal frequency, copied from the product specifications rather
/// than from the model, so a model change cannot make this gate agree with
/// itself.
const LFCLK_HZ: u64 = 32_768;

/// Descriptor directories scanned for `nrf52840_rtc` instances. The onboarding
/// set is included because it is imported Renode data that a customer can
/// actually load.
const CHIP_DIRS: &[&str] = &["configs/chips", "configs/chips/onboarding"];

// RTC register offsets (PS §6.21.13).
const TASKS_START: u64 = 0x000;
const TASKS_CLEAR: u64 = 0x008;
const PRESCALER: u64 = 0x508;
const COUNTER: u64 = 0x504;

fn chip_files_declaring_an_rtc() -> Vec<(PathBuf, ChipDescriptor)> {
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
                .any(|p| matches!(p.r#type.as_str(), "nrf52840_rtc" | "nrf52_rtc"))
            {
                out.push((path, chip));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "no chip descriptor declares an nRF RTC; did the model type change?"
    );
    out
}

fn build_bus(chip_path: &std::path::Path) -> SystemBus {
    let chip = ChipDescriptor::from_file(chip_path)
        .unwrap_or_else(|e| panic!("load {}: {e}", chip_path.display()));
    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("assemble {}: {e}", chip_path.display()))
}

/// Start an RTC at PRESCALER 0 from a clear counter, walk `cycles` CPU cycles,
/// then read COUNTER the way firmware does.
fn counter_after(bus: &mut SystemBus, base: u64, cycles: u64) -> u32 {
    bus.write_u32(base + PRESCALER, 0).unwrap();
    bus.write_u32(base + TASKS_CLEAR, 1).unwrap();
    bus.write_u32(base + TASKS_START, 1).unwrap();
    for _ in 0..cycles {
        let _ = bus.tick_peripherals_fully_forced();
    }
    bus.read_u32(base + COUNTER).unwrap()
}

/// Behaviour check: COUNTER counts LFCLK edges, so after `cycles` CPU cycles
/// it reads `cycles * 32768 / cpu_hz`. `cycles = cpu_hz / 1024` makes the
/// expected value exactly 32 for every core, including the 16 MHz nRF51 and
/// the 128 MHz nRF5340, where the un-attached model would read 8 or 64.
#[test]
fn nrf_rtc_counts_lfclk_not_cpu_cycles() {
    let mut wrong: Vec<String> = Vec::new();
    for (path, chip) in chip_files_declaring_an_rtc() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            chip.cpu_hz > 0,
            "{name}: descriptor must declare cpu_hz — the RTC ratio depends on it"
        );
        let cycles = chip.cpu_hz / 1024;
        let expected =
            u32::try_from(u128::from(cycles) * u128::from(LFCLK_HZ) / u128::from(chip.cpu_hz))
                .expect("fits u32");

        let mut bus = build_bus(&path);
        let mut checked = 0usize;
        for p in &chip.peripherals {
            if !matches!(p.r#type.as_str(), "nrf52840_rtc" | "nrf52_rtc") {
                continue;
            }
            let got = counter_after(&mut bus, p.base_address, cycles);
            if got != expected {
                wrong.push(format!(
                    "{name} `{}`: after {cycles} CPU cycles at {} Hz the RTC reads {got}, \
                     expected {expected}",
                    p.id, chip.cpu_hz
                ));
            }
            checked += 1;
        }
        assert!(checked > 0, "{name}: RTC instance disappeared");
    }
    assert!(
        wrong.is_empty(),
        "RTC COUNTER does not follow LFCLK (32.768 kHz) on {} instance(s); the LFCLK \
         ratio is not derived from cpu_hz:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

/// The 64 MHz nRF52 family is the byte-identical control: attaching cpu_hz
/// must not move a part whose core already matches the model's default.
#[test]
fn nrf52840_rtc_rate_is_unchanged_at_64_mhz() {
    let path = root("configs/chips/nrf52840.yaml");
    let mut bus = build_bus(&path);
    // 62_500 CPU cycles / 1953.125 = exactly 32 LFCLK ticks.
    assert_eq!(counter_after(&mut bus, 0x4000_b000, 62_500), 32);
}
