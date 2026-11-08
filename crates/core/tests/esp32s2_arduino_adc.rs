// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32 `analogRead()` on the ESP32-S2 converts through the
//! RTC-domain SENS controller.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l5.elf` is the Arduino matrix L5
//! sketch (`validation/arduino-matrix/sketches/L5_adc`) built for PlatformIO
//! `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It uses the L0
//! build's partition table (the two are byte-identical). The sketch sets
//! `analogReadResolution(12)`, reads A0 (GPIO1 = ADC1_CH0) three times, and
//! prints `LW_L5_OK` only for the exact twin code 2048 (the S2 SENS 13-bit
//! mid-scale 0x1000, shifted to 12 bits by the core's `mapResolution`). It
//! prints `LW_L5_VAL <v0> <v1>` either way.
//!
//! Without SENS the firmware never reaches `setup()`: IDF's boot-time
//! `adc_hw_calibration` constructor busy-polls `SENS_MEAS1_DONE_SAR`.

use labwired_core::peripherals::esp32s3::sens::Esp32s3Sens;
use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boot the L5 fixture on the matrix S2 system, optionally with `millivolts`
/// on ADC1_CH0, and return UART0 once the sketch has printed its values.
fn run_l5(ch0_millivolts: Option<u16>) -> String {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    if let Some(mv) = ch0_millivolts {
        let idx = bus
            .find_peripheral_index_by_name("sens")
            .expect("sens window");
        let sens = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32s3Sens>())
            .expect("sens is Esp32s3Sens");
        sens.set_channel_input(0, mv);
    }
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l5.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(&elf, &mut bus, &mut cpu, &flash, Some(&table))
        .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    // Run until the LW_L5_VAL line is complete (it ends in a newline).
    for _ in 0..12_000_000u64 {
        machine.step().expect("step");
        let out = uart.lock().unwrap();
        if let Some(p) = out.windows(10).position(|w| w == b"LW_L5_VAL ") {
            if out[p..].contains(&b'\n') {
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    text
}

#[test]
fn analog_read_completes_with_the_fixed_mid_scale_code() {
    let text = run_l5(None);
    assert!(text.contains("LW_L5_OK"), "no LW_L5_OK; UART0: {text:?}");
    assert!(text.contains("LW_L5_VAL 2048 2048"), "UART0: {text:?}");
}

#[test]
fn analog_read_follows_the_voltage_on_adc1_ch0() {
    // 1000 mV on the S2's 13-bit scale = 1000 * 8191 / 3300 = 2482, which
    // the core maps to 12 bits as 2482 >> 1 = 1241. A 12-bit model would give
    // 1240. The sketch's exact-2048 oracle then reports FAIL, as it should.
    let text = run_l5(Some(1000));
    assert!(text.contains("LW_L5_VAL 1241 1241"), "UART0: {text:?}");
    assert!(!text.contains("LW_L5_OK"), "UART0: {text:?}");
}
