// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Mega 2560: real Arduino-core firmware (examples/arduino-mega-2560-io,
//! built for arduino:avr:mega) runs on the ATmega2560 model and reaches the
//! outside world: the LED pins land on ports E/G/F/B/H, analogRead(A8)/(A9)
//! convert ADC8/ADC9 through ADCSRB.MUX5, millis() keeps time off Timer0 and
//! Serial prints through USART0. The sketch is linked for the 3-byte-PC part, so
//! booting it at all also covers EIND/EICALL, RAMPZ/ELPM and the 2560 vectors.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CPU_HZ: u64 = 16_000_000;

/// Sketch LED order -> (port model, bit): D3 D4 D5 A2 A1 A0 D13 D12 D8 D7 D6 D2.
const PADS: [(&str, u8); 12] = [
    ("porte", 5),
    ("portg", 5),
    ("porte", 3),
    ("portf", 2),
    ("portf", 1),
    ("portf", 0),
    ("portb", 7),
    ("portb", 6),
    ("porth", 5),
    ("porth", 4),
    ("porth", 3),
    ("porte", 4),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Rig {
    machine: Machine<Avr>,
    serial: Arc<Mutex<Vec<u8>>>,
}

fn boot() -> Rig {
    let yaml = root().join("examples/arduino-mega-2560-io/system.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-mega-2560-io.elf"))
        .expect("missing fixture; build examples/arduino-mega-2560-io");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    Rig {
        machine: Machine::new(cpu, bus),
        serial,
    }
}

impl Rig {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.serial.lock().unwrap()).into_owned()
    }

    /// Step until the next complete `k=..` line, returning it and the cycle
    /// count when its newline went out.
    fn next_line(&mut self) -> (String, u64) {
        let start = self.text().matches('\n').count();
        for _ in 0..8_000_000u32 {
            self.machine.step().expect("step");
            let text = self.text();
            if text.matches('\n').count() > start && text.ends_with('\n') {
                let line = text.lines().last().unwrap_or_default().to_string();
                return (line, self.machine.total_cycles);
            }
        }
        panic!("sketch stopped printing; serial so far: {:?}", self.text());
    }

    fn next_k_line(&mut self) -> (String, u64) {
        loop {
            let (line, cycles) = self.next_line();
            if line.starts_with("k=") {
                return (line, cycles);
            }
        }
    }

    fn pad(&self, port: &str, pin: u8) -> Option<bool> {
        let idx = self
            .machine
            .bus
            .find_peripheral_index_by_name(port)
            .unwrap_or_else(|| panic!("atmega2560 registers '{port}'"));
        self.machine.bus.peripherals[idx].dev.read_gpio_output(pin)
    }
}

fn field(line: &str, key: &str) -> u64 {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(key))
        .unwrap_or_else(|| panic!("no {key} in {line:?}"))
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a number in {line:?}"))
}

#[test]
fn serial_banner_comes_out_of_usart0() {
    let mut rig = boot();
    let (line, _) = rig.next_line();
    assert_eq!(line.trim_end(), "MEGA READY");
}

#[test]
fn each_led_lands_on_its_mega_port_pin() {
    let mut rig = boot();
    for _ in 0..12 {
        let (line, _) = rig.next_k_line();
        let k = field(&line, "k=") as usize;
        for (i, (port, bit)) in PADS.iter().enumerate() {
            assert_eq!(
                rig.pad(port, *bit),
                Some(i == k),
                "step k={k}: {port} bit {bit} (LED {i}) in {line:?}"
            );
        }
    }
}

#[test]
fn millis_tracks_simulated_time() {
    let mut rig = boot();
    let mut prev: Option<(u64, u64)> = None;
    for _ in 0..5 {
        let (line, cycles) = rig.next_k_line();
        let ms = field(&line, "ms=");
        // millis() never runs ahead of the simulated clock, and lags it by at
        // most a couple of Timer0 overflows (1.024 ms each) plus print time.
        let sim_ms = cycles * 1000 / CPU_HZ;
        assert!(ms <= sim_ms + 1, "ms={ms} ahead of sim {sim_ms} ms");
        assert!(sim_ms - ms <= 30, "ms={ms} lags sim {sim_ms} ms");
        if let Some((pms, pcycles)) = prev {
            assert!(
                (100..=102).contains(&(ms - pms)),
                "100 ms cadence: {pms} -> {ms}"
            );
            let dt = (cycles - pcycles) * 1000 / CPU_HZ;
            assert!((99..=102).contains(&dt), "line cadence {dt} ms of sim time");
        }
        prev = Some((ms, cycles));
    }
}

#[test]
fn a8_and_a9_read_adc8_and_adc9() {
    let mut rig = boot();
    let (centred, _) = rig.next_k_line();
    assert_eq!(field(&centred, "a8="), 512, "2.5 V: 2500 * 1024 / 5000");
    assert_eq!(field(&centred, "a9="), 512);
    assert_eq!(field(&centred, "a0="), 512, "ADC0 is undriven (mid-rail)");

    rig.machine
        .set_input_on("pot_a8", "position", 100.0)
        .expect("A8 to 100 %");
    rig.machine
        .set_input_on("pot_a9", "position", 0.0)
        .expect("A9 to 0 %");
    rig.next_k_line();
    let (moved, _) = rig.next_k_line();
    assert_eq!(
        field(&moved, "a8="),
        1023,
        "A8 at the 5 V rail is full scale"
    );
    assert_eq!(field(&moved, "a9="), 0, "A9 at ground");
    assert_eq!(
        field(&moved, "a0="),
        512,
        "A0 is still undriven ADC0: MUX5 cleared again, not ADC8"
    );

    rig.machine
        .set_input_on("pot_a9", "position", 25.0)
        .expect("A9 to 25 %");
    rig.next_k_line();
    let (quarter, _) = rig.next_k_line();
    assert_eq!(field(&quarter, "a8="), 1023);
    assert_eq!(field(&quarter, "a9="), 256, "1.25 V: 1250 * 1024 / 5000");
}

#[test]
fn indirect_calls_go_through_eicall() {
    let mut rig = boot();
    for _ in 0..4 {
        let (line, _) = rig.next_k_line();
        let k = field(&line, "k=");
        let want = if k & 1 == 1 { 3 * (k + 1) } else { 2 * (k + 1) };
        assert_eq!(field(&line, "f="), want, "{line:?}");
    }
}
