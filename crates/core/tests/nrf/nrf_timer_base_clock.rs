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
//! | nRF54L15    | 128 MHz  | 128 MHz on TIMER00, 32 MHz on TIMER10, |
//! |             |          | 16 MHz on TIMER2x                      |
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
//! Five checks:
//! 1. Every `nrf52840_timer` instance in every nRF chip descriptor either
//!    declares the expected `peripheral_hz`, or its base clock equals the core
//!    clock so no key is needed. A new timer instance must make that choice.
//! 2. Behaviour: after a known number of CPU cycles the captured counter equals
//!    `cycles * base_hz / cpu_hz` exactly, read the way firmware does it
//!    (TASKS_CAPTURE[0] then CC[0]).
//! 3. nRF5340 TIMER0..2 expose all six capture/compare channels, not the
//!    model's default four.
//! 4. The nRF54L full profiles declare every TIMER instance the vendor SVD
//!    defines, at the SVD's base address and IRQ.
//! 5. nRF52832/33/40 and the imported nRF52840 profile expose four CC channels
//!    on TIMER0..2 and six on TIMER3..4, matching the Nordic product specs.

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

        // nRF54L: TIMER00 on HF PLL (the core clock), TIMER10 on HFXO
        // (32 MHz), TIMER2x on PCLK16M.
        ("nrf54l05" | "nrf54l10", "timer10") => Some(32_000_000),
        ("nrf54l15", "timer00") => Some(128_000_000),
        ("nrf54l15", "timer10") => Some(32_000_000),
        ("nrf54l15", "timer20" | "timer21" | "timer22" | "timer23" | "timer24") => Some(16_000_000),

        // nRF54LM20A adds SPIM00 and a fourth GPIO port; its TIMER00 and
        // TIMER23/24 are the same instances the shared dtsi defines on L15.
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

/// Secure TIMER instances the vendor SVD defines: `(descriptor id, base, irq)`.
///
/// Parsed from the SVD so the expected instance list, base addresses and IRQs
/// come from the vendor file, not from a table written next to the descriptor
/// under test. Only `<peripheral>` names ending in `_S` are read; the `_NS`
/// aliases sit at the same bases and the descriptors use the secure view.
fn svd_secure_timers(svd: &std::path::Path) -> Vec<(String, u64, u32)> {
    let text =
        std::fs::read_to_string(svd).unwrap_or_else(|e| panic!("read {}: {e}", svd.display()));
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut base: Option<u64> = None;
    let mut irq: Option<u32> = None;
    let mut in_interrupt = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("<peripheral") {
            name = None;
            base = None;
            irq = None;
            in_interrupt = false;
        } else if let Some(v) = t
            .strip_prefix("<name>")
            .and_then(|s| s.strip_suffix("</name>"))
        {
            // The first <name> in the peripheral block is the instance name;
            // later ones (registers, fields) must not overwrite it.
            if name.is_none() {
                name = Some(v.trim().to_string());
            }
        } else if let Some(v) = t
            .strip_prefix("<baseAddress>")
            .and_then(|s| s.strip_suffix("</baseAddress>"))
        {
            let v = v.trim();
            let hex = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X"));
            base = u64::from_str_radix(hex.unwrap_or(v), 16).ok();
        } else if t == "<interrupt>" {
            in_interrupt = true;
        } else if in_interrupt {
            if let Some(v) = t
                .strip_prefix("<value>")
                .and_then(|s| s.strip_suffix("</value>"))
            {
                irq = v.trim().parse().ok();
                in_interrupt = false;
            }
        } else if t == "</peripheral>" {
            if let (Some(n), Some(b)) = (name.take(), base) {
                if n.starts_with("GLOBAL_TIMER") && n.ends_with("_S") {
                    let id = n
                        .trim_start_matches("GLOBAL_")
                        .trim_end_matches("_S")
                        .to_ascii_lowercase();
                    out.push((id, b, irq.expect("SVD TIMER without an interrupt value")));
                }
            }
            in_interrupt = false;
        }
    }
    out.sort();
    out
}

/// Descriptor check: the nRF54L full profiles declare every TIMER instance the
/// vendor SVD defines.
///
/// `svd_conformance` only verifies the instances a descriptor already declares,
/// so a MISSING TIMER is invisible to every other gate until firmware touches
/// it and takes a `MemoryViolation`. `nrf54l_05_10_15.dtsi` and the vendored
/// `nrf54l15.svd` both define seven on this part: TIMER00, TIMER10 and
/// TIMER20..24. The L15 profile declared four of them until 2026-11-24.
///
/// The instance list, bases and IRQs are read from the SVD. `cc-num` is not
/// expressible in an SVD (every instance derives from `GLOBAL_TIMER00_NS` with
/// `<dim>8</dim>`), so it comes from the devicetree `cc-num` property below.
#[test]
fn nrf54l_profiles_declare_every_timer_the_devicetree_defines() {
    // dtsi `cc-num`: TIMER10 has eight capture/compare channels, the rest six.
    const EXPECTED_CC: &[(&str, u64)] = &[
        ("timer00", 6),
        ("timer10", 8),
        ("timer20", 6),
        ("timer21", 6),
        ("timer22", 6),
        ("timer23", 6),
        ("timer24", 6),
    ];

    for (chip_file, svd_file) in [
        ("nrf54l15.yaml", "nrf54l15.svd"),
        ("nrf54lm20a.yaml", "nrf54lm20a.svd"),
    ] {
        let svd = root(&format!("tests/fixtures/real_world/{svd_file}"));
        let expected = svd_secure_timers(&svd);
        assert_eq!(
            expected.len(),
            EXPECTED_CC.len(),
            "{}: expected {} TIMER instances in the SVD, parsed {}",
            svd.display(),
            EXPECTED_CC.len(),
            expected.len()
        );

        let chip_path = root(&format!("configs/chips/{chip_file}"));
        let chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("load {chip_file}: {e}"));

        let mut findings: Vec<String> = Vec::new();
        for (id, base, irq) in &expected {
            let Some(p) = chip.peripherals.iter().find(|p| p.id == *id) else {
                findings.push(format!("{id}: not declared; SVD has it at {base:#X}"));
                continue;
            };
            if p.r#type != "nrf52840_timer" {
                findings.push(format!(
                    "{id}: type `{}`, expected nrf52840_timer",
                    p.r#type
                ));
            }
            if p.base_address != *base {
                findings.push(format!(
                    "{id}: base {:#X}, SVD says {base:#X}",
                    p.base_address
                ));
            }
            if p.irq != Some(*irq) {
                findings.push(format!("{id}: irq {:?}, SVD says {irq}", p.irq));
            }
            let num_cc = EXPECTED_CC
                .iter()
                .find(|(cc_id, _)| cc_id == id)
                .map(|(_, cc)| *cc);
            if p.config.get("num_cc").and_then(|v| v.as_u64()) != num_cc {
                findings.push(format!(
                    "{id}: num_cc {:?}, devicetree says {num_cc:?}",
                    p.config.get("num_cc")
                ));
            }
        }
        for p in chip
            .peripherals
            .iter()
            .filter(|p| p.r#type == "nrf52840_timer")
        {
            if !expected.iter().any(|(id, ..)| *id == p.id) {
                findings.push(format!(
                    "{}: unexpected timer instance; the SVD does not define it",
                    p.id
                ));
            }
        }

        assert!(
            findings.is_empty(),
            "{chip_file} timer estate disagrees with {svd_file}:\n  {}",
            findings.join("\n  ")
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
        ("nrf54l15.yaml", "timer00", 0x5005_5000, 640, 640), // 128 MHz / 128 MHz
        ("nrf54l15.yaml", "timer10", 0x5008_5000, 640, 160), // 32 MHz / 128 MHz
        ("nrf54l15.yaml", "timer20", 0x500C_A000, 640, 80), // 16 MHz / 128 MHz
        ("nrf54l15.yaml", "timer23", 0x500C_D000, 640, 80), // 16 MHz / 128 MHz
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

/// Nordic nRF52832/33/40 Product Specification, TIMER configuration tables:
/// TIMER0..2 have CC[0..3], and TIMER3..4 have CC[0..5]. The onboarding
/// nRF52840 profile is included because its imported peripheral list is
/// maintained separately from the shipped chip descriptor.
#[test]
fn nrf52_profiles_match_the_timer_compare_channel_estate() {
    const CHIP_FILES: &[&str] = &[
        "nrf52832.yaml",
        "nrf52833.yaml",
        "nrf52840.yaml",
        "onboarding/nrf52840.yaml",
    ];

    for chip_file in CHIP_FILES {
        let chip_path = root(&format!("configs/chips/{chip_file}"));
        let chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("load {chip_file}: {e}"));
        let manifest = SystemManifest {
            chip: chip_path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let mut bus = SystemBus::from_config(&chip, &manifest)
            .unwrap_or_else(|e| panic!("assemble {chip_file}: {e}"));

        let mut checked = 0usize;
        for timer in chip
            .peripherals
            .iter()
            .filter(|p| p.r#type == "nrf52840_timer")
        {
            let expected_cc = match timer.id.as_str() {
                "timer0" | "timer1" | "timer2" => 4usize,
                "timer3" | "timer4" => 6usize,
                other => panic!("{chip_file}: add `{other}` to the Nordic TIMER CC audit"),
            };
            let configured_cc = timer
                .config
                .get("num_cc")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(4);
            assert_eq!(
                configured_cc, expected_cc,
                "{chip_file} `{}`: descriptor/model channel count disagrees with the Nordic PS",
                timer.id
            );

            // Probe the highest implemented CC register and the first absent
            // one through the assembled bus. The model must retain the former
            // and ignore the latter, so the descriptor count is the hardware
            // surface the firmware actually sees.
            let last_cc = timer.base_address + CC0 + ((expected_cc as u64 - 1) * 4);
            let first_absent_cc = timer.base_address + CC0 + (expected_cc as u64 * 4);
            let marker = 0x5A00_0000 | expected_cc as u32;
            bus.write_u32(last_cc, marker).unwrap();
            assert_eq!(
                bus.read_u32(last_cc).unwrap(),
                marker,
                "{chip_file} `{}`: highest implemented CC register did not retain its value",
                timer.id
            );
            bus.write_u32(first_absent_cc, marker).unwrap();
            assert_eq!(
                bus.read_u32(first_absent_cc).unwrap(),
                0,
                "{chip_file} `{}`: first absent CC register is exposed",
                timer.id
            );
            checked += 1;
        }
        assert!(checked > 0, "{chip_file}: no nRF52 TIMER instances found");
    }
}
