// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! End-to-end test for the ESP32-S3 "OpenAI deck" example: builds
//! examples/openai-deck-s3, boots the ELF on the S3 fast-boot path with an
//! SH1107 attached to I2C0, asserts the OLED framebuffer renders (non-blank),
//! then drives a key GPIO high via `set_gpio_input` and asserts the KEY press
//! host-protocol line appears on serial.
//!
//! Mirrors `e2e_i2c_tmp102.rs` (build-the-example + fast_boot + manual step
//! loop) and the SH1107 framebuffer readback pattern from `esp32s3_oled_profile`.

#![cfg(feature = "esp32s3-fixtures")]

use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Sh1107;
use labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Cpu, SimulationError};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// SH1107 address the firmware drives (SA0=high; 0x3C is taken by the S3
/// wiring's default SSD1306 and the controller dispatches to the first match).
const OLED_ADDR: u8 = 0x3D;
/// KEY1 sits on GPIO4 (see examples/openai-deck-s3/src/main.rs KEY_PINS).
const KEY1_GPIO: u8 = 4;

fn firmware_path() -> PathBuf {
    PathBuf::from(
        "../../examples/openai-deck-s3/target/xtensa-esp32s3-none-elf/release/openai-deck-s3",
    )
}

fn ensure_firmware_built() -> PathBuf {
    let elf = firmware_path();
    let src = PathBuf::from("../../examples/openai-deck-s3/src/main.rs");
    if elf.exists() {
        if let (Ok(elf_meta), Ok(src_meta)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
            if elf_meta.modified().unwrap() >= src_meta.modified().unwrap() {
                return elf;
            }
        }
    }
    let status = Command::new("cargo")
        .args(["+esp", "build", "--release"])
        .current_dir("../../examples/openai-deck-s3")
        .status()
        .expect("cargo +esp build (is the ESP toolchain installed and ~/export-esp.sh sourced?)");
    assert!(status.success(), "openai-deck-s3 build failed");
    assert!(elf.exists(), "ELF not found at {elf:?} after build");
    elf
}

/// Read back the SH1107 GDDRAM attached at `OLED_ADDR` on I2C0.
fn oled_lit_pixels(bus: &SystemBus) -> usize {
    let idx = bus
        .find_peripheral_index_by_name("i2c0")
        .expect("S3 bus exposes i2c0");
    bus.peripherals[idx]
        .dev
        .as_any()
        .expect("i2c0 downcastable")
        .downcast_ref::<Esp32s3I2c>()
        .expect("i2c0 is the ESP32-S3 command-list controller")
        .attached_slaves()
        .iter()
        .filter(|s| s.address() == OLED_ADDR)
        .find_map(|s| s.as_any().and_then(|a| a.downcast_ref::<Sh1107>()))
        .expect("SH1107 attached at OLED_ADDR")
        .lit_pixels()
}

/// Drive a GPIO input level on the S3 `gpio` peripheral.
fn set_key(bus: &mut SystemBus, pin: u8, level: bool) {
    let idx = bus
        .find_peripheral_index_by_name("gpio")
        .expect("S3 bus exposes gpio");
    assert!(
        bus.peripherals[idx].dev.set_gpio_input(pin, level),
        "gpio must accept an injected input on pin {pin}"
    );
}

#[test]
fn openai_deck_s3_renders_and_reports_key_press() {
    let elf_path = ensure_firmware_built();
    let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    // Wire the SH1107 the SAME way the app does: from the manifest's declared
    // `external_devices`, through the generic `attach_esp32_external_devices`
    // factory — NOT an out-of-band `attach_i2c_slave`. A pass therefore proves the
    // real product path (app/CLI) renders the deck, not a test-only bypass.
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: openai-deck-s3
chip: esp32s3
external_devices:
  - id: oled
    type: oled-sh1107
    connection: i2c0
    config:
      i2c_address: 0x3D
  - id: knob
    type: potentiometer
    connection: sar_adc_s3
    config:
      channel: 0
      initial_position_pct: 60.0
  - id: fader
    type: potentiometer
    connection: sar_adc_s3
    config:
      channel: 1
      initial_position_pct: 40.0
"#,
    )
    .expect("parse inline deck manifest");
    labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        .expect("attach SH1107 from manifest");
    bus.refresh_peripheral_index();

    // Sink USB-Serial-JTAG for the host-protocol assertions.
    let jtag = Arc::new(Mutex::new(Vec::<u8>::new()));
    for p in bus.peripherals.iter_mut() {
        if let Some(any) = p.dev.as_any_mut() {
            if let Some(uj) = any.downcast_mut::<UsbSerialJtag>() {
                uj.set_sink(Some(jtag.clone()), false);
            }
        }
    }

    let icache_backing = wiring.icache_backing.clone();
    let dcache_backing = wiring.dcache_backing.clone();
    let mut cpu = wiring.cpu;

    fast_boot(
        &elf_bytes,
        &mut bus,
        &mut cpu,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(icache_backing),
            dcache_backing: Some(dcache_backing),
        },
    )
    .expect("fast_boot");

    let observers: Vec<Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let cfg = labwired_core::SimulationConfig::default();

    // Phase 1: run until the OLED renders (non-blank) — the firmware inits I2C0,
    // brings up the SH1107, and paints the title + key grid. Budget generously.
    const RENDER_MAX_STEPS: u64 = 40_000_000;
    let mut steps: u64 = 0;
    let mut lit_after_render = 0usize;
    while steps < RENDER_MAX_STEPS {
        match cpu.step(&mut bus, &observers, &cfg) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        let _ = bus.tick_peripherals_with_costs();
        steps += 1;
        if steps % 100_000 == 0 {
            let lit = oled_lit_pixels(&bus);
            if lit > 0 {
                lit_after_render = lit;
                break;
            }
        }
    }
    assert!(
        lit_after_render > 0,
        "SH1107 framebuffer stayed blank after {steps} steps; firmware did not render"
    );

    // Phase 2: press KEY1 (GPIO4 → high) and run until the firmware polls the
    // edge and emits its host-protocol line.
    set_key(&mut bus, KEY1_GPIO, true);
    const PRESS_MAX_STEPS: u64 = 20_000_000;
    let start = steps;
    let mut saw_key = false;
    while steps - start < PRESS_MAX_STEPS {
        match cpu.step(&mut bus, &observers, &cfg) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(e) => panic!("simulator error at pc=0x{:08x}: {e}", cpu.get_pc()),
        }
        let _ = bus.tick_peripherals_with_costs();
        steps += 1;
        if steps % 50_000 == 0 {
            let text = String::from_utf8_lossy(&jtag.lock().unwrap()).to_string();
            if text.contains("KEY") {
                saw_key = true;
                break;
            }
        }
    }

    let text = String::from_utf8_lossy(&jtag.lock().unwrap()).to_string();
    assert!(
        saw_key && text.contains("KEY1 PRESS action=SLOT1"),
        "expected a 'KEY1 PRESS action=SLOT1' line after pressing GPIO{KEY1_GPIO}; serial:\n{text}"
    );

    // The two pots are seeded from the manifest (knob 60 %, fader 40 %) through
    // the SENS ADC, and the firmware's analogRead maps them: knob → temperature,
    // fader → max-tokens. Exact values prove the manifest→factory→SENS→firmware
    // path, not a mid-scale placeholder (0x800 → temp=1.00 max_tokens=2048).
    assert!(
        text.contains("PARAMS knob_raw=2457 fader_raw=1638 temp=1.20 max_tokens=1638"),
        "expected the pot-driven PARAMS line (knob 60 %, fader 40 %); serial:\n{text}"
    );

    let lit_after_press = oled_lit_pixels(&bus);
    eprintln!(
        "OPENAI_DECK_S3 render_steps={} lit_after_render={} lit_after_press={} serial={:?}",
        start, lit_after_render, lit_after_press, text
    );
}
