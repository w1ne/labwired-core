// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Uno R3: every ATmega328P port reaches the outside world.
//!
//! Before PORTC/PORTD had bus-side models, only PORTB (D8-D13) was visible: a
//! canvas LED on D7 never lit, a button on D2 was never read, and analogRead
//! returned 512 whatever the potentiometer said. This runs real Arduino-core
//! firmware (examples/arduino-uno-io) through the example's system.yaml and
//! checks each port from the outside: D7 toggles on the `portd` model, a press
//! on D2 changes what the sketch prints and lights D13, and moving the knob on
//! A0 moves the counts it converts.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Rig {
    machine: Machine<Avr>,
    serial: Arc<Mutex<Vec<u8>>>,
}

fn boot() -> Rig {
    boot_with(|yaml| yaml)
}

/// Boot the example with its system.yaml rewritten by `edit` first.
fn boot_with(edit: impl FnOnce(String) -> String) -> Rig {
    let yaml = root().join("examples/arduino-uno-io/system.yaml");
    let text = edit(std::fs::read_to_string(&yaml).expect("read system.yaml"));
    let manifest = SystemManifest::from_yaml(&text).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-uno-io.elf"))
        .expect("missing fixture; build examples/arduino-uno-io");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new();
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

    /// Step until `count` further complete `D2=.. A0=..` lines have printed,
    /// and return the last one.
    fn next_lines(&mut self, count: usize) -> String {
        let start = self.text().matches('\n').count();
        for _ in 0..3_000_000u32 {
            self.machine.step().expect("step");
            let text = self.text();
            if text.matches('\n').count() >= start + count && text.ends_with('\n') {
                return text.lines().last().unwrap_or_default().to_string();
            }
        }
        panic!("sketch stopped printing; serial so far: {:?}", self.text());
    }

    fn port_output(&self, port: &str, pin: u8) -> Option<bool> {
        let idx = self
            .machine
            .bus
            .find_peripheral_index_by_name(port)
            .unwrap_or_else(|| panic!("atmega328p registers '{port}'"));
        self.machine.bus.peripherals[idx].dev.read_gpio_output(pin)
    }
}

fn field(line: &str, key: &str) -> u32 {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(key))
        .unwrap_or_else(|| panic!("no {key} in {line:?}"))
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a number in {line:?}"))
}

#[test]
fn d7_output_is_visible_on_portd() {
    let mut rig = boot();
    rig.next_lines(1);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..6 {
        rig.next_lines(1);
        seen.insert(rig.port_output("portd", 7));
    }
    assert_eq!(
        seen,
        [Some(false), Some(true)].into_iter().collect(),
        "D7 must be driven and toggle on the portd model"
    );
}

#[test]
fn a_press_on_d2_reaches_the_sketch_and_lights_d13() {
    let mut rig = boot();
    let released = rig.next_lines(2);
    assert_eq!(
        field(&released, "D2="),
        1,
        "released pull-up button reads HIGH"
    );
    assert_eq!(rig.port_output("portb", 5), Some(false));

    rig.machine
        .set_input_on("btn_d2", "pressed", 1.0)
        .expect("press D2");
    let pressed = rig.next_lines(2);
    assert_eq!(field(&pressed, "D2="), 0, "pressed button pulls D2 LOW");
    assert_eq!(
        rig.port_output("portb", 5),
        Some(true),
        "the sketch mirrors the press onto LED_BUILTIN"
    );
}

#[test]
fn the_potentiometer_on_a0_moves_analog_read() {
    let mut rig = boot();
    let centred = field(&rig.next_lines(2), "A0=");

    rig.machine
        .set_input_on("knob", "position", 0.0)
        .expect("knob to 0 %");
    let bottom = field(&rig.next_lines(2), "A0=");

    rig.machine
        .set_input_on("knob", "position", 100.0)
        .expect("knob to 100 %");
    let top = field(&rig.next_lines(2), "A0=");

    // 3.3 V rail on a 5 V reference: 3300 * 1024 / 5000 = 675 at full travel,
    // half that centred. Exact, because the converter is exact.
    assert_eq!(bottom, 0, "knob at 0 % converts to 0");
    assert_eq!(
        top, 675,
        "knob at 100 % is the 3.3 V rail on a 5 V reference"
    );
    assert_eq!(centred, 337, "the knob starts centred at 50 %");
}

/// End to end: `config.supply_mv` on the pot survives `attach` and drives what
/// the firmware's analogRead returns. With the pot across the Uno's 5 V rail,
/// full travel is the converter's full scale (1023), not the 3.3 V curve's 675.
#[test]
fn a_pot_on_the_5v_rail_reads_full_scale_through_attach() {
    let mut rig = boot_with(|yaml| {
        let edited = yaml.replace(
            "      channel: 0\n",
            "      channel: 0\n      supply_mv: 5000\n",
        );
        assert_ne!(
            edited, yaml,
            "example yaml still has the knob's channel line"
        );
        edited
    });
    let centred = field(&rig.next_lines(2), "A0=");
    rig.machine
        .set_input_on("knob", "position", 100.0)
        .expect("knob to 100 %");
    let top = field(&rig.next_lines(2), "A0=");
    rig.machine
        .set_input_on("knob", "position", 0.0)
        .expect("knob to 0 %");
    let bottom = field(&rig.next_lines(2), "A0=");
    assert_eq!(top, 1023, "5 V on a 5 V reference is full scale");
    assert_eq!(centred, 512, "2.5 V centred: 2500 * 1024 / 5000");
    assert_eq!(bottom, 0);
}
