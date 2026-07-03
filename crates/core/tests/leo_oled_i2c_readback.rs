// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Regression guard for the OLED-blank-in-embed bug: the ESP32-C3 leo
// air-quality lab attaches its SSD1306 to the command-list `Esp32c3I2c`
// controller (not the generic STM32 `I2c`). The playground/embed reads the
// framebuffer through `WasmSimulator::get_ssd1306_framebuffer`, which enumerates
// I²C slaves on the named peripheral. That accessor originally only understood
// the generic `I2c`, so on the C3 it returned "not an I2C controller" and the
// OLED rendered blank even though the device was present and being drawn to.
//
// This test locks in the two facts the fix depends on:
//   1. Building the leo bus from its committed system.yaml attaches an SSD1306
//      slave (address 0x3C) to the C3 I²C0 controller.
//   2. The `Esp32c3I2c::attached_slaves()` accessor surfaces it and its
//      framebuffer is readable via the same downcast chain `inspect.rs` uses.
// The existing `e2e_leo_airquality` test proves the firmware renders a
// non-blank frame into that same SSD1306 instance.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1306;
use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
use std::path::PathBuf;

#[test]
fn leo_oled_is_readable_through_esp32c3_i2c() {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32c3-leo-airquality/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load leo manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load esp32c3 chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build leo bus");

    let idx = bus
        .find_peripheral_index_by_name("i2c0")
        .expect("leo config must expose an 'i2c0' peripheral");
    let any = bus.peripherals[idx]
        .dev
        .as_any()
        .expect("i2c0 peripheral must support downcasting");

    // On the C3 the controller is Esp32c3I2c, NOT the generic I2c — this is the
    // whole point of the readback bug.
    let c3 = any
        .downcast_ref::<Esp32c3I2c>()
        .expect("leo i2c0 must be the ESP32-C3 command-list controller");

    // The exact enumeration path `get_ssd1306_framebuffer` now uses for the C3.
    let oled = c3
        .attached_slaves()
        .iter()
        .filter(|d| d.address() == 0x3C)
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1306>()))
        .expect("an SSD1306 must be attached at 0x3C on the C3 i2c0 bus");

    // Framebuffer is readable (128x64 -> 1024 bytes, page-major). Blank at boot
    // is fine here; e2e_leo_airquality asserts the firmware fills it.
    let fb = oled.framebuffer();
    assert_eq!(
        fb.len(),
        1024,
        "SSD1306 GDDRAM framebuffer must be 128x64/8 = 1024 bytes"
    );
}
