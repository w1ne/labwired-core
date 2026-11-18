// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega2560 16-bit Timer/Counter1, 3, 4 and 5 under avr-gcc firmware.
//!
//! The committed fixture (`tests/fixtures/avr/atmega2560-timers.c`, built with
//! avr-gcc for `-mmcu=atmega2560`) names its handlers with avr-libc's vector
//! macros, so the program addresses come from the toolchain's DS2549 Table 14-1,
//! not from the simulator. It runs Timer1 in CTC with OCR1A as TOP plus a
//! compare C match, Timer3 in 8-bit fast PWM with its overflow interrupt and
//! three PWM pads, Timer4 in fast PWM with ICR4 as TOP (ICF4 at TOP) and three
//! PWM pads, and Timer5 in CTC at 1 kHz. It prints the interrupt counts after
//! 100 Timer5 ticks and leaves the PWM running.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CPU_HZ: u64 = 16_000_000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn boot() -> (Machine<Avr>, Arc<Mutex<Vec<u8>>>) {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega2560.yaml")).expect("chip");
    let manifest = SystemManifest::from_yaml(
        r#"
name: "mega-timers"
chip: "atmega2560.yaml"
external_devices: []
"#,
    )
    .expect("manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/atmega2560-timers.elf"))
        .expect("missing tests/fixtures/avr/atmega2560-timers.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new_model(AvrModel::Atmega2560);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    (Machine::new(cpu, bus), serial)
}

fn pad(m: &Machine<Avr>, port: &str, pin: u8) -> bool {
    let idx = m.bus.find_peripheral_index_by_name(port).expect("port");
    m.bus.peripherals[idx]
        .dev
        .read_gpio_output(pin)
        .unwrap_or(false)
}

fn count(line: &str, key: &str) -> u32 {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("{key} missing in {line:?}"))
        .parse()
        .expect("number")
}

#[test]
fn mega_16_bit_timers_interrupt_at_their_rates_and_drive_their_pwm_pads() {
    let (mut m, serial) = boot();
    // 100 ms of Timer5 ticks plus the print.
    let line = loop {
        m.step().expect("step");
        let text = String::from_utf8_lossy(&serial.lock().unwrap()).into_owned();
        if text.ends_with('\n') {
            break text;
        }
        assert!(m.cpu.cycles < CPU_HZ / 5, "no report by 200 ms: {text:?}");
    };
    // 100 ms at 2 kHz (Timer1 A and C), 976.6 Hz (Timer3 TOV at TOP),
    // 1 kHz (Timer4 ICF at TOP, Timer5 compare A). Counts are read the
    // instant Timer5's 100th tick lands, so a source whose next event is
    // only clocks away may still be one short.
    assert_eq!(count(&line, "T5A"), 100, "{line}");
    assert!((199..=200).contains(&count(&line, "T1A")), "{line}");
    assert!((199..=200).contains(&count(&line, "T1C")), "{line}");
    assert!((97..=98).contains(&count(&line, "T3OVF")), "{line}");
    assert!((99..=100).contains(&count(&line, "T4CAPT")), "{line}");

    // PWM pads: fast PWM duty is (OCR + 1) / (TOP + 1) (DS2549 17.9.3);
    // pins per 13.3.5 (port E) and 13.3.8 (port H). The window is a whole
    // number of periods of both timers (Timer3 64 * 256 = 16384 clocks,
    // Timer4 8 * 2000 = 16000 clocks; lcm 2 048 000 = 128 ms), so no partial
    // period biases the duty.
    let cases = [
        ("OC3A PE3", ("porte", 3), 65.0 / 256.0),
        ("OC3B PE4", ("porte", 4), 129.0 / 256.0),
        ("OC3C PE5", ("porte", 5), 193.0 / 256.0),
        ("OC4A PH3", ("porth", 3), 500.0 / 2000.0),
        ("OC4B PH4", ("porth", 4), 1000.0 / 2000.0),
        ("OC4C PH5", ("porth", 5), 1500.0 / 2000.0),
    ];
    let start = m.cpu.cycles;
    let end = start + 2_048_000;
    let mut high = [0u64; 6];
    let mut t = start;
    while m.cpu.cycles < end {
        let now: Vec<bool> = cases.iter().map(|(_, (p, b), _)| pad(&m, p, *b)).collect();
        m.step().expect("step");
        let dt = m.cpu.cycles - t;
        t = m.cpu.cycles;
        for (h, on) in high.iter_mut().zip(now) {
            if on {
                *h += dt;
            }
        }
    }
    for ((name, _, want), h) in cases.iter().zip(high) {
        let got = h as f64 / (t - start) as f64;
        assert!((got - want).abs() < 0.01, "{name} duty {got}, want {want}");
    }
}
