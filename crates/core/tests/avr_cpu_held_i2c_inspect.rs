// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AVR moves I²C kits off the bus onto the CPU TWI model. After that move the
//! kits still simulate, but a bus-only inspect walk cannot see them. This test
//! pins the contract that `Machine::inspect` / `Machine::display_artifact`
//! rejoin those CPU-held kits so an LCD on an Uno stays named and readable.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::inspect::InspectOpts;
use labwired_core::peripherals::components::lcd1602::Lcd1602;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::Cpu;
use labwired_core::Machine;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Drive one nibble the way LiquidCrystal_I2C pulses Enable.
fn pulse_nibble(dev: &mut dyn I2cDevice, nibble: u8, rs: bool) {
    const PIN_RS: u8 = 0x01;
    const PIN_ENABLE: u8 = 0x04;
    const PIN_BACKLIGHT: u8 = 0x08;
    let base = (nibble << 4) | PIN_BACKLIGHT | if rs { PIN_RS } else { 0 };
    dev.write(base | PIN_ENABLE);
    dev.write(base);
}

fn send_byte(dev: &mut dyn I2cDevice, byte: u8, rs: bool) {
    pulse_nibble(dev, byte >> 4, rs);
    pulse_nibble(dev, byte & 0x0F, rs);
}

fn paint_hi(dev: &mut dyn I2cDevice) {
    send_byte(dev, 0x0C, false); // display on
    send_byte(dev, b'H', true);
    send_byte(dev, b'i', true);
}

fn uno_with_lcd() -> (SystemBus, Avr) {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).expect("chip");
    let manifest = SystemManifest::from_yaml(
        r#"
name: "uno-lcd"
chip: "atmega328p.yaml"
external_devices:
  - id: "DS1"
    type: "lcd1602"
    connection: "i2c"
    config:
      i2c_address: 0x27
"#,
    )
    .expect("manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let mut cpu = Avr::new();
    for dev in bus.take_i2c_slaves("i2c") {
        cpu.push_i2c_slave(dev);
    }
    assert_eq!(cpu.i2c_slaves.len(), 1, "LCD must land on the CPU TWI model");
    {
        let slave = &mut *cpu.i2c_slaves[0];
        paint_hi(slave);
        let lcd = slave
            .as_any()
            .and_then(|a| a.downcast_ref::<Lcd1602>())
            .expect("lcd1602");
        assert!(lcd.text().starts_with("Hi"), "paint must land: {:?}", lcd.text());
    }
    (bus, cpu)
}

#[test]
fn cpu_held_lcd_is_invisible_to_bus_only_inspect() {
    let (bus, _cpu) = uno_with_lcd();
    let opts = InspectOpts::default();
    let ids: Vec<_> = bus
        .inspect_devices(None, &opts)
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert!(
        !ids.iter().any(|id| id == "DS1"),
        "bus-only inspect must not invent a bus-resident LCD after take; got {ids:?}"
    );
}

#[test]
fn machine_inspect_names_cpu_held_lcd_and_reads_its_text() {
    let (bus, cpu) = uno_with_lcd();
    let machine = Machine::new(cpu, bus);
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let devices = machine.inspect(None, &opts).devices;
    let lcd = devices
        .iter()
        .find(|d| d.id == "DS1")
        .unwrap_or_else(|| panic!("DS1 missing from Machine::inspect; got {devices:?}"));
    assert!(lcd.declared, "join must match the external_devices declaration");
    assert_eq!(lcd.device_type.as_deref(), Some("lcd1602"));
    assert_eq!(lcd.attachment.transport, "i2c");
    assert_eq!(lcd.attachment.bus.as_deref(), Some("i2c"));
    assert_eq!(lcd.attachment.address, Some(0x27));

    let art = lcd
        .artifacts
        .iter()
        .find(|a| a.kind == "text_display")
        .expect("text_display artifact");
    let text = art
        .meta
        .get("text")
        .and_then(|v| v.as_str())
        .expect("meta.text");
    assert!(text.starts_with("Hi"), "inspect must show painted text, got {text:?}");

    let door = machine
        .bus
        .display_artifact_with_extra("DS1", &opts, &mut |f| {
            machine.cpu.for_each_attached_device(f);
        })
        .expect("display_artifact_with_extra must reach the CPU-held LCD");
    assert_eq!(door.kind, "text_display");
    let door_text = door.meta.get("text").and_then(|v| v.as_str()).unwrap();
    assert!(door_text.starts_with("Hi"), "display_artifact text {door_text:?}");
}
