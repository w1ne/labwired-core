// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Uno: Timer1 input capture on ICP1 (D8) and the ADC temperature
//! sensor, from a stock Arduino-core sketch.
//!
//! The committed sketch (`tests/fixtures/avr/arduino-uno-capture.cpp`, built
//! with PlatformIO `atmelavr`, board `uno`) prints three lines:
//! - `TEMP raw=` is mux 0x08 against the internal 1.1 V. The model gives the
//!   datasheet's typical 314 mV at +25 °C, 314 * 1024 / 1100 = 292.
//! - `POLL` drives D8 high 200 us after TCNT1 = 0 at clk/8, so ICF1 is set and
//!   ICR1 holds about 400 ticks plus the `digitalWrite` overhead.
//! - `ISR` lets `tone(8, 1000)` toggle D8 from Timer2's ISR while
//!   `TIMER1_CAPT` records eight rising edges: seven 1000 us periods.
//!
//! Before the capture unit and the sensor were modelled the same ELF printed
//! `TEMP raw=0`, `POLL icf=0 icr1=0` and `ISR n=0`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_to_done() -> String {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).expect("chip");
    let manifest =
        SystemManifest::from_file(root().join("configs/systems/arduino-uno.yaml")).expect("uno");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-uno-capture.elf"))
        .expect("missing tests/fixtures/avr/arduino-uno-capture.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    let mut machine = Machine::new(cpu, bus);
    let text = || String::from_utf8_lossy(&serial.lock().unwrap()).into_owned();
    for _ in 0..20_000_000u32 {
        machine.step().expect("step");
        if text().contains("DONE\r\n") {
            return text();
        }
    }
    panic!("sketch did not finish; serial so far: {:?}", text());
}

fn field<'a>(text: &'a str, line: &str, key: &str) -> &'a str {
    let l = text
        .lines()
        .find(|l| l.starts_with(line))
        .unwrap_or_else(|| panic!("no {line} line in {text:?}"));
    let v = l
        .split(' ')
        .find_map(|kv| kv.strip_prefix(key))
        .unwrap_or_else(|| panic!("no {key} in {l:?}"));
    v.trim_end()
}

#[test]
fn input_capture_and_temperature_sensor_run_a_stock_sketch() {
    let text = run_to_done();

    assert_eq!(field(&text, "TEMP", "raw="), "292", "{text}");

    assert_eq!(field(&text, "POLL", "icf="), "1", "{text}");
    let icr: u16 = field(&text, "POLL", "icr1=").parse().unwrap();
    assert!((400..=420).contains(&icr), "ICR1 {icr}: 200 us at clk/8 plus overhead");

    assert_eq!(field(&text, "ISR", "n="), "8", "{text}");
    let periods: Vec<u16> = field(&text, "ISR", "periods_us=")
        .split(',')
        .map(|p| p.parse().unwrap())
        .collect();
    assert_eq!(periods.len(), 7);
    for p in periods {
        assert!((998..=1002).contains(&p), "tone(8, 1000) period {p} us");
    }
}
