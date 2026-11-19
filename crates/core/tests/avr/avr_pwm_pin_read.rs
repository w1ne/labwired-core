// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Firmware reads a pin that `analogWrite` drives and sees the waveform.
//!
//! The committed sketch (`tests/fixtures/avr/arduino-uno-pwm-read.cpp`)
//! polls `PIND`/`PINB` on D6 (OC0A), D9 (OC1A) and D3 (OC2B) for 20 ms each,
//! then counts PCINT22 (D6) and INT1 rising (D3) over a 20 ms `delay()`.
//! The pads already carried the waveform, but `PINx` and the pin-change /
//! external-interrupt sampler read the `PORTx` latch, so all five counts were
//! 0. On silicon the synchronizer samples the pad (ATmega328P datasheet,
//! section 14.2.4).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::node::{build_node, NodeFirmware};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn core_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The number after `key` on the serial line that starts with `line`.
fn field(serial: &str, line: &str, key: &str) -> u32 {
    let l = serial
        .lines()
        .find(|l| l.starts_with(line))
        .unwrap_or_else(|| panic!("no {line:?} line; serial={serial:?}"));
    let rest = &l[l.find(key).unwrap_or_else(|| panic!("no {key:?} in {l:?}")) + key.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("bad {key:?} in {l:?}"))
}

#[test]
fn pin_reads_and_pin_interrupts_follow_the_pwm_waveform() {
    let root = core_root();
    let chip: ChipDescriptor = {
        let y = std::fs::read_to_string(root.join("configs/chips/atmega328p.yaml")).unwrap();
        serde_yaml::from_str(&y).expect("chip yaml")
    };
    let system: SystemManifest = {
        let y = std::fs::read_to_string(root.join("configs/systems/arduino-uno.yaml")).unwrap();
        serde_yaml::from_str(&y).expect("system yaml")
    };
    let elf = std::fs::read(root.join("tests/fixtures/avr/arduino-uno-pwm-read.elf"))
        .expect("missing tests/fixtures/avr/arduino-uno-pwm-read.elf");
    let mut machine =
        build_node("uno", &chip, &system, NodeFirmware::Elf(elf)).expect("build_node");
    let sink = Arc::new(Mutex::new(Vec::new()));
    machine
        .attach_uart_tx_sink(sink.clone(), false)
        .expect("uart sink");

    let mut serial = String::new();
    for step in 0..3_000_000u32 {
        if let Err(e) = machine.step() {
            panic!("step {step} pc={:#x}: {e:?}", machine.get_pc());
        }
        if step % 1024 == 0 {
            serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
            if serial.contains("R369 DONE") {
                break;
            }
        }
    }
    assert!(serial.contains("R369 DONE"), "serial={serial:?}");

    // 20 ms windows: Timer0 ~976.6 Hz (19.5 periods), Timer1/Timer2
    // phase-correct ~490.2 Hz (9.8 periods). Two edges per period.
    for (pin, lo, hi, duty) in [("D6", 37, 41, 25), ("D9", 18, 22, 75), ("D3", 18, 22, 50)] {
        let edges = field(&serial, pin, "edges=");
        let pct = field(&serial, pin, "high%=");
        assert!((lo..=hi).contains(&edges), "{pin} edges={edges}");
        assert!(pct.abs_diff(duty) <= 4, "{pin} high%={pct}, want ~{duty}");
    }
    let pc = field(&serial, "PCINT22", "n=");
    assert!((37..=42).contains(&pc), "PCINT22 n={pc}");
    let int1 = field(&serial, "INT1", "n=");
    assert!((9..=11).contains(&int1), "INT1 n={int1}");
}
