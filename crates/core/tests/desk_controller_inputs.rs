// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Public part ids/channels must reach the Uno's ADC and GPIO, not merely
//! appear in catalog metadata. Uses the committed real Arduino-core sketch
//! that prints analogRead(A0) and digitalRead(D2), with its wiring replaced.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::peripherals::components::declarative_analog::DeclarativeAnalogKit;
use labwired_core::sim_input::SimInput;
use labwired_core::Machine;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bus(devices: &str) -> SystemBus {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).unwrap();
    let manifest = SystemManifest::from_yaml(&format!(
        "name: desk-inputs\nchip: atmega328p\nexternal_devices:\n{devices}"
    ))
    .unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

struct Rig {
    machine: Machine<Avr>,
    serial: Arc<Mutex<Vec<u8>>>,
}

impl Rig {
    fn new(devices: &str) -> Self {
        let image =
            labwired_loader::load_elf(&root().join("tests/fixtures/avr/arduino-uno-io.elf"))
                .unwrap();
        let mut cpu = Avr::new();
        cpu.load_program_image(&image);
        let serial = Arc::new(Mutex::new(Vec::new()));
        cpu.set_serial_sink(serial.clone());
        Self {
            machine: Machine::new(cpu, bus(devices)),
            serial,
        }
    }

    fn reading(&mut self, key: &str) -> u32 {
        let initial = self
            .serial
            .lock()
            .unwrap()
            .iter()
            .filter(|b| **b == b'\n')
            .count();
        for _ in 0..3_000_000 {
            self.machine.step().unwrap();
            let serial = self.serial.lock().unwrap();
            // Two lines avoid interpreting a sample acquired before a stimulus
            // but still being transmitted when it was applied.
            if serial.last() == Some(&b'\n')
                && serial.iter().filter(|b| **b == b'\n').count() >= initial + 2
            {
                let text = String::from_utf8_lossy(&serial);
                return text
                    .lines()
                    .last()
                    .unwrap()
                    .split_whitespace()
                    .find_map(|p| p.strip_prefix(key))
                    .unwrap()
                    .parse()
                    .unwrap();
            }
        }
        panic!(
            "Arduino fixture did not produce a fresh reading: {:?}",
            self.serial.lock().unwrap()
        );
    }
}

#[test]
fn joystick_public_channels_reach_both_adc_axes_and_the_push_contact() {
    let mut rig = Rig::new("  - id: joy\n    type: joystick\n    connection: adc\n    config: {x_channel: 1, y_channel: 0, sw_pin: PD2, supply_mv: 5000}\n");
    let inputs = rig.machine.bus.list_inputs();
    for key in ["x", "y", "pressed"] {
        assert_eq!(
            inputs
                .iter()
                .filter(|(id, channel)| id == "joy" && channel.key == key)
                .count(),
            1
        );
    }
    assert!(!inputs
        .iter()
        .any(|(id, ch)| id.contains(':') || ch.key == "position"));
    assert_eq!(rig.reading("A0="), 512);
    for (position, expected) in [(-1.0, 0), (0.5, 768), (1.0, 1023), (0.0, 512)] {
        rig.machine.set_input_on("joy", "y", position).unwrap();
        assert_eq!(rig.reading("A0="), expected);
    }
    rig.machine.set_input_on("joy", "x", -1.0).unwrap();
    let x = rig
        .machine
        .bus
        .analog_inputs
        .iter()
        .find(|a| a.channel == 1)
        .unwrap();
    assert_eq!(x.source.output_mv(), 0);
    assert_eq!(rig.reading("A0="), 512, "moving X must not move Y");
    assert_eq!(rig.reading("D2="), 1);
    rig.machine.set_input_on("joy", "pressed", 1.0).unwrap();
    assert_eq!(rig.reading("D2="), 0);
    assert!(rig.machine.set_input_on("joy", "y", 1.1).is_err());
    assert!(rig.machine.set_input_on("joy:y", "position", 50.0).is_err());
}

#[test]
fn joystick_unwired_outputs_are_not_offered_as_driveable_inputs() {
    let mut bus = bus("  - id: joy\n    type: joystick\n    connection: adc\n    config: {y_channel: 0, supply_mv: 5000}\n");
    let inputs = bus.list_inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].0, "joy");
    assert_eq!(inputs[0].1.key, "y");
    assert!(bus.set_input(Some("joy"), "x", 0.0).is_err());
    assert!(bus.set_input(Some("joy"), "pressed", 1.0).is_err());
}

#[test]
fn acs712_current_reaches_analog_read_and_crosses_the_2a_threshold() {
    let mut rig = Rig::new("  - id: current\n    type: acs712\n    connection: adc\n    config: {channel: 0, supply_mv: 5000}\n");
    assert_eq!(rig.reading("A0="), 512, "zero current is VCC/2");
    for (amps, expected_adc) in [(0.5, 531), (2.5, 606), (-2.5, 417)] {
        rig.machine
            .set_input_on("current", "current", amps)
            .unwrap();
        let adc = rig.reading("A0=");
        assert_eq!(adc, expected_adc);
        let measured_ma = ((i64::from(adc) * 5000 / 1023 - 2500) * 1000 / 185).abs();
        assert!((measured_ma as f64 - amps.abs() * 1000.0).abs() < 30.0);
        assert_eq!(measured_ma > 2000, amps.abs() > 2.0);
    }
    assert!(rig.machine.set_input_on("current", "current", 5.1).is_err());
}

#[test]
fn acs712_ratiometric_transfer_matches_datasheet_across_the_signed_range() {
    let kit =
        DeclarativeAnalogKit::from_yaml(labwired_config::embedded_device_yaml("acs712").unwrap())
            .unwrap();
    let mut sensor = kit.build(0).unwrap();
    for supply in [4500.0, 5000.0, 5500.0] {
        sensor.set_supply_mv(supply, Some(5000.0)).unwrap();
        for milliamp in (-5000..=5000).step_by(25) {
            let amps = f64::from(milliamp) / 1000.0;
            sensor.set_input("current", amps).unwrap();
            let expected = ((2500.0 + 185.0 * amps) * supply / 5000.0).round() as u16;
            assert_eq!(
                sensor.output_mv(),
                expected,
                "supply {supply}, current {amps}"
            );
        }
    }
}

#[test]
fn limit_switch_no_nc_share_one_press_and_reach_the_sketch() {
    let mut rig = Rig::new("  - id: upper\n    type: limit-switch\n    connection: gpio\n    config: {no_pin: PD2, nc_pin: PD3, common_high: false}\n");
    let portd = rig
        .machine
        .bus
        .find_peripheral_index_by_name("portd")
        .unwrap();
    assert_eq!(
        rig.machine
            .bus
            .list_inputs()
            .iter()
            .filter(|(id, c)| id == "upper" && c.key == "pressed")
            .count(),
        1
    );
    assert_eq!(rig.reading("D2="), 1);
    assert_eq!(
        rig.machine.bus.peripherals[portd].dev.read_gpio_input(3),
        Some(false)
    );
    rig.machine.set_input_on("upper", "pressed", 1.0).unwrap();
    // Both poles update immediately, before any instruction executes.
    assert_eq!(
        rig.machine.bus.peripherals[portd].dev.read_gpio_input(2),
        Some(false)
    );
    assert_eq!(
        rig.machine.bus.peripherals[portd].dev.read_gpio_input(3),
        Some(true)
    );
    assert_eq!(rig.reading("D2="), 0);
    rig.machine.set_input_on("upper", "pressed", 0.0).unwrap();
    assert_eq!(rig.reading("D2="), 1);
    assert_eq!(
        rig.machine.bus.peripherals[portd].dev.read_gpio_input(3),
        Some(false)
    );
}

#[test]
fn limit_switch_nc_only_and_high_common_have_the_opposite_polarity() {
    for (key, common_high, rest, pressed) in [
        ("nc_pin", false, false, true),
        ("no_pin", true, false, true),
        ("nc_pin", true, true, false),
    ] {
        let mut bus = bus(&format!("  - id: stop\n    type: limit-switch\n    connection: gpio\n    config: {{{key}: PD2, common_high: {common_high}}}\n"));
        let portd = bus.find_peripheral_index_by_name("portd").unwrap();
        assert_eq!(bus.peripherals[portd].dev.read_gpio_input(2), Some(rest));
        bus.set_input(Some("stop"), "pressed", 1.0).unwrap();
        assert_eq!(bus.peripherals[portd].dev.read_gpio_input(2), Some(pressed));
    }
}
