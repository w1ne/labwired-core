// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino `tone()` on an Uno, which is Timer2 in CTC mode.
//!
//! The committed sketch (`tests/fixtures/avr/arduino-uno-tone.cpp`, built
//! against ArduinoCore-avr 1.8.6) calls `tone(6, 700, 80)` and then
//! `tone(6, 700, 240)`. The core's compare-match A ISR toggles D6 (PD6) and
//! `noTone()` leaves the pin idle. At 16 MHz the core picks OCR2A = 177 and
//! clk/64, so the pin flips every 178 * 64 CPU cycles — about 702 Hz — for
//! 112 and 336 toggles (about 79 ms and 239 ms, ratio 3).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;

const CPU_HZ: u64 = 16_000_000;
/// Edges farther apart than this belong to different tones.
const BURST_GAP: u64 = CPU_HZ * 5 / 1000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn portd6(bus: &SystemBus) -> bool {
    let idx = bus
        .find_peripheral_index_by_name("portd")
        .expect("atmega328p registers portd");
    bus.peripherals[idx]
        .dev
        .read_gpio_output(6)
        .unwrap_or(false)
}

fn bursts(edges: &[u64]) -> Vec<&[u64]> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..edges.len() {
        if edges[i] - edges[i - 1] > BURST_GAP {
            out.push(&edges[start..i]);
            start = i;
        }
    }
    if start < edges.len() {
        out.push(&edges[start..]);
    }
    out
}

fn median_spacing(edges: &[u64]) -> u64 {
    let mut gaps: Vec<u64> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.len() >= 3, "need several toggles, got {}", edges.len());
    gaps.sort_unstable();
    gaps[gaps.len() / 2]
}

#[test]
fn tone_on_d6_is_about_700_hz_for_80_ms_then_240_ms() {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).expect("chip");
    let manifest =
        SystemManifest::from_file(root().join("configs/systems/arduino-uno.yaml")).expect("uno");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-uno-tone.elf"))
        .expect("missing tests/fixtures/avr/arduino-uno-tone.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    let cfg = SimulationConfig::default();

    let mut level = portd6(&bus);
    let mut edges: Vec<u64> = Vec::new();
    let horizon = CPU_HZ * 700 / 1000;
    let idle_tail = CPU_HZ * 40 / 1000;
    while cpu.cycles < horizon {
        cpu.step(&mut bus, &[], &cfg)
            .unwrap_or_else(|e| panic!("step pc={:#x} cycles={}: {e:?}", cpu.pc, cpu.cycles));
        let now = portd6(&bus);
        if now != level {
            edges.push(cpu.cycles);
            level = now;
        }
        if edges.len() >= 2 {
            let groups = bursts(&edges);
            if groups.len() >= 2 && cpu.cycles.saturating_sub(*edges.last().unwrap()) > idle_tail {
                break;
            }
        }
    }

    let groups = bursts(&edges);
    assert_eq!(
        groups.len(),
        2,
        "expected two tones on D6, saw {} edges over {} cycles (pin now {level})",
        edges.len(),
        cpu.cycles,
    );

    // (OCR2A + 1) * prescaler. 700 Hz is the requested tone; the integer OCR
    // the core actually programs is 702 Hz. One short first gap (the sticky
    // compare flag set while tone() is still writing OCR2A) must not move
    // the median.
    let expected_gap = 178 * 64;
    let mut durations = [0u64; 2];
    for (i, burst) in groups.iter().enumerate() {
        let gap = median_spacing(burst);
        let freq = CPU_HZ as f64 / (2.0 * gap as f64);
        durations[i] = burst.last().unwrap() - burst.first().unwrap();
        let ms = durations[i] as f64 * 1000.0 / CPU_HZ as f64;
        assert!(
            (gap as i64 - expected_gap as i64).unsigned_abs() <= expected_gap / 50,
            "tone {i}: median gap {gap} cycles is {freq:.1} Hz, want ~702 (gap {expected_gap})"
        );
        assert!((650.0..=750.0).contains(&freq), "tone {i}: {freq:.1} Hz");
        let edges_n = burst.len();
        let expect_edges = if i == 0 { 112 } else { 336 };
        assert!(
            (expect_edges - 8..expect_edges + 8).contains(&edges_n),
            "tone {i}: {edges_n} edges, want about {expect_edges} ({ms:.1} ms)"
        );
    }

    let first_ms = durations[0] as f64 * 1000.0 / CPU_HZ as f64;
    let second_ms = durations[1] as f64 * 1000.0 / CPU_HZ as f64;
    let ratio = second_ms / first_ms;
    let hz = |burst: &[u64]| CPU_HZ as f64 / (2.0 * median_spacing(burst) as f64);
    println!(
        "D6 tone: {:.1} Hz for {:.1} ms ({} edges), then {:.1} Hz for {:.1} ms ({} edges), ratio {:.3}",
        hz(groups[0]),
        first_ms,
        groups[0].len(),
        hz(groups[1]),
        second_ms,
        groups[1].len(),
        ratio
    );
    assert!(
        (70.0..=95.0).contains(&first_ms),
        "first tone lasted {first_ms:.1} ms, want ~80"
    );
    assert!(
        (210.0..=270.0).contains(&second_ms),
        "second tone lasted {second_ms:.1} ms, want ~240"
    );
    assert!(
        (2.8..=3.2).contains(&ratio),
        "duration ratio {ratio:.3} (second/first), want ~3"
    );
    assert!(!level, "D6 must sit low once both tones have finished");
}
