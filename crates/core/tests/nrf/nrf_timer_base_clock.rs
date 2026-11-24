// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every nRF TIMER must count its OWN base clock, not the core clock.
//!
//! `Nrf52Timer` advances one base tick per CPU cycle when the descriptor does
//! not declare `config: { peripheral_hz }`. That is only correct when the TIMER
//! base clock equals the core clock. It does on nRF51 (16 MHz core, 16 MHz
//! TIMER), but not on the parts this gate covers:
//!
//! | part        | core     | TIMER base clock                       |
//! |-------------|----------|----------------------------------------|
//! | nRF52832    | 64 MHz   | 16 MHz on TIMER0..2                    |
//! | nRF5340     | 128 MHz  | 16 MHz (PCLK16M) on TIMER0..2          |
//! | nRF54L05/10 | 128 MHz  | 32 MHz (HFXO) on TIMER10               |
//! | nRF54L15    | 128 MHz  | 32 MHz on TIMER10, 16 MHz on TIMER2x   |
//! | nRF54LM20A  | 128 MHz  | 128 MHz on TIMER00, 32 MHz on TIMER10, |
//! |             |          | 16 MHz on TIMER2x                      |
//!
//! Without the key those timers run 8x or 4x fast, silently: a UART console
//! still prints and a compare still fires, just at the wrong instant.
//!
//! Sources: `TIMERxx_PCLK_MHZ` in the Nordic MDK headers
//! (`nrf54l15_application_peripherals.h`, `nrf54lm20a_application_peripherals.h`,
//! `nrf54l05/10_application_peripherals.h`), the Zephyr devicetree `clocks`
//! property (`nrf54l_05_10_15.dtsi`: TIMER00 -> hfpll, TIMER10 -> hfxo
//! 32 MHz, TIMER2x -> PCLK16M), and nRF5340 PS v1.5 (TIMER counts PCLK16M).
//!
//! Two checks:
//! 1. Every `nrf52840_timer` instance in every nRF chip descriptor either
//!    declares the expected `peripheral_hz`, or its base clock equals the core
//!    clock so no key is needed. A new timer instance must make that choice.
//! 2. Behaviour: after a known number of CPU cycles the captured counter equals
//!    `cycles * base_hz / cpu_hz` exactly, read the way firmware does it
//!    (TASKS_CAPTURE[0] then CC[0]).
//! 3. nRF5340 TIMER0..2 expose all six capture/compare channels, not the
//!    model's default four.

use crate::common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

/// Chip descriptors under the gate, by file name. Every TIMER instance in each
/// one must be named in `expected_timer_hz`.
const NRF_CHIPS: &[&str] = &[
    "nrf51822.yaml",
    "nrf52832.yaml",
    "nrf52833.yaml",
    "nrf52840.yaml",
    "nrf5340.yaml",
    "nrf54l05.yaml",
    "nrf54l10.yaml",
    "nrf54l15.yaml",
    "nrf54lm20a.yaml",
];

/// TIMER base clock from the part's MDK header / product specification.
fn expected_timer_hz(chip: &str, timer_id: &str) -> Option<u64> {
    match (chip, timer_id) {
        // nRF51/nRF52: 16 MHz TIMER base clock (PS §6.30).
        ("nrf51822" | "nrf52832" | "nrf52833" | "nrf52840", _) => Some(16_000_000),

        // nRF5340 application core: PCLK16M on all three timers.
        ("nrf5340", "timer0" | "timer1" | "timer2") => Some(16_000_000),

        // nRF54L: TIMER10 on HFXO (32 MHz), TIMER2x on PCLK16M.
        ("nrf54l05" | "nrf54l10", "timer10") => Some(32_000_000),
        ("nrf54l15", "timer10") => Some(32_000_000),
        ("nrf54l15", "timer20" | "timer21" | "timer22") => Some(16_000_000),

        // nRF54LM20A adds TIMER00 on HF PLL (128 MHz) and TIMER23/24.
        ("nrf54lm20a", "timer00") => Some(128_000_000),
        ("nrf54lm20a", "timer10") => Some(32_000_000),
        ("nrf54lm20a", "timer20" | "timer21" | "timer22" | "timer23" | "timer24") => {
            Some(16_000_000)
        }

        _ => None,
    }
}

fn build_bus(chip_file: &str) -> SystemBus {
    let chip_path = root(&format!("configs/chips/{chip_file}"));
    let chip =
        ChipDescriptor::from_file(&chip_path).unwrap_or_else(|e| panic!("load {chip_file}: {e}"));
    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("assemble {chip_file}: {e}"))
}

const TASKS_START: u64 = 0x000;
const TASKS_CLEAR: u64 = 0x00C;
const TASKS_CAPTURE0: u64 = 0x040;
const TASKS_CAPTURE5: u64 = 0x054;
const BITMODE: u64 = 0x508;
const PRESCALER: u64 = 0x510;
const CC0: u64 = 0x540;
const CC5: u64 = 0x554;

/// Put a timer into 32-bit mode at PRESCALER 0 and start it from zero.
fn start_timer(bus: &mut SystemBus, timer_base: u64) {
    bus.write_u32(timer_base + BITMODE, 3).unwrap(); // 32-bit counter
    bus.write_u32(timer_base + PRESCALER, 0).unwrap(); // no prescaler
    bus.write_u32(timer_base + TASKS_CLEAR, 1).unwrap();
    bus.write_u32(timer_base + TASKS_START, 1).unwrap();
}

/// Capture the running counter after `cycles` CPU cycles, exactly as firmware
/// does: strobe TASKS_CAPTURE[0], then read CC[0]. The forced walk is used so
/// the measurement is the model's per-cycle advance, independent of whether
/// this build attached a scheduler cycle clock.
fn counter_after(bus: &mut SystemBus, timer_base: u64, cycles: u64) -> u32 {
    start_timer(bus, timer_base);

    for _ in 0..cycles {
        let _ = bus.tick_peripherals_fully_forced();
    }

    bus.write_u32(timer_base + TASKS_CAPTURE0, 1).unwrap();
    bus.read_u32(timer_base + CC0).unwrap()
}

/// Descriptor check: no nRF TIMER may fall back to the core clock unless the
/// two clocks are genuinely equal.
#[test]
fn every_nrf_timer_declares_a_base_clock_when_it_differs_from_the_core() {
    for chip_file in NRF_CHIPS {
        let chip_path = root(&format!("configs/chips/{chip_file}"));
        let chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("load {chip_file}: {e}"));

        let mut checked = 0usize;
        for p in &chip.peripherals {
            if p.r#type != "nrf52840_timer" {
                continue;
            }
            let expected = expected_timer_hz(&chip.name, &p.id).unwrap_or_else(|| {
                panic!(
                    "{chip_file}: timer `{}` is not in this gate's table. Add its MDK \
                     `TIMERxx_PCLK_MHZ` value to expected_timer_hz so a base clock \
                     cannot be forgotten.",
                    p.id
                )
            });
            let declared = p.config.get("peripheral_hz").and_then(|v| v.as_u64());
            match declared {
                Some(hz) => assert_eq!(
                    hz, expected,
                    "{chip_file}: timer `{}` declares peripheral_hz={hz}, MDK says {expected}",
                    p.id
                ),
                None => assert_eq!(
                    expected, chip.cpu_hz,
                    "{chip_file}: timer `{}` has no `peripheral_hz`, which is only correct \
                     when its base clock ({expected} Hz) equals the core clock ({} Hz)",
                    p.id, chip.cpu_hz
                ),
            }
            checked += 1;
        }
        assert!(
            checked > 0,
            "{chip_file}: no `nrf52840_timer` instances found; did the model type change?"
        );
    }
}

/// Behaviour check: the captured counter is `cycles * base / cpu`, not
/// `cycles`. Each row runs 640 CPU cycles, so the expected count is exact.
#[test]
fn nrf_timers_count_their_base_clock_not_the_core_clock() {
    // (chip file, timer id for the message, base address, CPU cycles, expected count)
    const ROWS: &[(&str, &str, u64, u64, u32)] = &[
        ("nrf5340.yaml", "timer0", 0x5000_f000, 640, 80), // 16 MHz / 128 MHz
        ("nrf54l15.yaml", "timer10", 0x5008_5000, 640, 160), // 32 MHz / 128 MHz
        ("nrf54l15.yaml", "timer20", 0x500C_A000, 640, 80), // 16 MHz / 128 MHz
        ("nrf54lm20a.yaml", "timer00", 0x5005_5000, 640, 640), // 128 MHz / 128 MHz
        ("nrf54l05.yaml", "timer10", 0x5008_5000, 640, 160), // 32 MHz / 128 MHz
        ("nrf52832.yaml", "timer1", 0x4000_9000, 640, 160), // 16 MHz / 64 MHz
        ("nrf52832.yaml", "timer2", 0x4000_A000, 640, 160), // 16 MHz / 64 MHz
    ];

    for (chip_file, timer_id, base, cycles, expected) in ROWS {
        let mut bus = build_bus(chip_file);
        let got = counter_after(&mut bus, *base, *cycles);
        assert_eq!(
            got, *expected,
            "{chip_file} `{timer_id}`: after {cycles} CPU cycles the counter is {got}, \
             expected {expected} (base clock not applied?)"
        );
    }
}

/// Descriptor check: nRF5340 TIMER0..2 expose six capture/compare channels.
///
/// The model defaults to 4 channels, which silently drops CC[4]/CC[5] writes.
/// The vendored SVD (`configs/peripherals/nrf5340/timer0_s.yaml`) lists
/// TASKS_CAPTURE0..5 and EVENTS_COMPARE0..5; the MDK says `TIMER0_CC_NUM 6`.
#[test]
fn nrf5340_timers_expose_six_compare_channels() {
    const TIMERS: &[(u64, u32)] = &[
        (0x5000_f000, 8), // timer0, after 64 CPU cycles at 16/128 MHz
        (0x5001_0000, 8), // timer1
        (0x5001_1000, 8), // timer2
    ];

    for (base, expected_capture) in TIMERS {
        let mut bus = build_bus("nrf5340.yaml");
        start_timer(&mut bus, *base);

        // A write to CC[5] must stick.
        bus.write_u32(base + CC5, 0x1234_5678).unwrap();
        assert_eq!(
            bus.read_u32(base + CC5).unwrap(),
            0x1234_5678,
            "timer at {base:#x}: CC[5] write was dropped (num_cc < 6?)"
        );

        // TASKS_CAPTURE[5] must overwrite it with the counter value.
        for _ in 0..64 {
            let _ = bus.tick_peripherals_fully_forced();
        }
        bus.write_u32(base + TASKS_CAPTURE5, 1).unwrap();
        assert_eq!(
            bus.read_u32(base + CC5).unwrap(),
            *expected_capture,
            "timer at {base:#x}: TASKS_CAPTURE[5] did not capture (num_cc < 6?)"
        );
    }
}
