// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Executing oracle for the ESP32-S3 command-list I2C controller
//! (`esp32s3/i2c.rs`, `Esp32s3I2c`) AND display fidelity for the shipped
//! esp32s3-oled lab's SSD1306 panel.
//!
//! The S3 had GPIO/timer/IRQ-matrix walk differentials but no I2C executing
//! coverage, even though the S3 I2C command engine is ~byte-identical to the
//! C3's (which does have one). Here the FULL bus is assembled with
//! `configure_xtensa_esp32s3` — the same wiring the browser twin boots — which
//! attaches an SSD1306 @ 0x3C to `i2c0`. We then drive the command-list engine
//! exactly as firmware does, over real bus MMIO at `I2C0_BASE`: load a command
//! list (RSTART / WRITE / STOP) into CMD0.., stage the I2C bytes in the TX
//! FIFO, and kick `CTR.trans_start`. The controller executes the transaction
//! synchronously, ACKs the 0x3C address, and streams the payload into the
//! panel — after which we read the SSD1306 GDDRAM back and assert the panel
//! renders EXACTLY the expected content. A register-map, command-decode,
//! address-match, or GDDRAM-addressing regression fails here.
//!
//! Named `esp32s3_*` / `*_ssd1306_*` so the board-coverage ratchet discovers it
//! as both the S3 I2C oracle and the S3 display differential.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Ssd1306;
use labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::Bus;

// ESP32-S3 I2C0 register block base and command-list register map (offsets are
// base-relative; the bus router subtracts the base for us).
const I2C0_BASE: u64 = 0x6001_3000;
const CTR: u64 = 0x04;
const DATA: u64 = 0x1C; // TX/RX FIFO data register
const INT_RAW: u64 = 0x20;
const INT_CLR: u64 = 0x24;
const CMD0: u64 = 0x58; // CMD0..CMD7, stride 4

const CTR_TRANS_START: u32 = 1 << 5;
const TRANS_COMPLETE: u32 = 1 << 7;
const NACK: u32 = 1 << 10;

// Command-list opcodes (bits [13:11]); byte_num in bits [7:0].
const OP_WRITE: u32 = 1;
const OP_STOP: u32 = 2;
const OP_RSTART: u32 = 6;

const SSD1306_ADDR: u8 = 0x3C;
const CTRL_CMD_STREAM: u8 = 0x00; // Co=0, D/C#=0 -> command stream
const CTRL_DATA_STREAM: u8 = 0x40; // Co=0, D/C#=1 -> data stream

fn cmd(opcode: u32, byte_num: u8) -> u32 {
    ((opcode & 0x7) << 11) | u32::from(byte_num)
}

/// Assemble the same bus the browser twin boots. The SSD1306 @ 0x3C is wired
/// from the board manifest through `attach_esp32_external_devices` — the SAME
/// path the app/CLI use — not a hardcoded builder attach.
fn build_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: esp32s3-ssd1306
chip: esp32s3
external_devices:
  - id: oled
    type: oled-ssd1306
    connection: i2c0
    config:
      i2c_address: 0x3C
"#,
    )
    .expect("parse ssd1306 manifest");
    labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        .expect("attach SSD1306 from manifest");
    bus.refresh_peripheral_index();
    bus
}

fn read_ssd1306(bus: &SystemBus) -> &Ssd1306 {
    let idx = bus
        .find_peripheral_index_by_name("i2c0")
        .expect("S3 bus exposes i2c0");
    bus.peripherals[idx]
        .dev
        .as_any()
        .expect("i2c0 is downcastable")
        .downcast_ref::<Esp32s3I2c>()
        .expect("i2c0 is the S3 command-list controller")
        .attached_slaves()
        .iter()
        .filter(|s| s.address() == SSD1306_ADDR)
        .find_map(|s| s.as_any().and_then(|a| a.downcast_ref::<Ssd1306>()))
        .expect("SSD1306 attached @ 0x3C")
}

/// Drive one I2C write transaction (address + payload) through the command-list
/// engine at `I2C0_BASE` and return the completion INT_RAW word.
fn i2c_write_transaction(bus: &mut SystemBus, payload: &[u8]) -> u32 {
    // Command list: RSTART, WRITE (addr + payload bytes), STOP.
    let byte_num = (payload.len() + 1) as u8; // +1 for the address byte
    bus.write_u32(I2C0_BASE + CMD0, cmd(OP_RSTART, 0)).unwrap();
    bus.write_u32(I2C0_BASE + CMD0 + 4, cmd(OP_WRITE, byte_num))
        .unwrap();
    bus.write_u32(I2C0_BASE + CMD0 + 8, cmd(OP_STOP, 0))
        .unwrap();

    // TX FIFO: 7-bit address + write bit, then the payload.
    bus.write_u32(I2C0_BASE + DATA, u32::from(SSD1306_ADDR) << 1)
        .unwrap();
    for &b in payload {
        bus.write_u32(I2C0_BASE + DATA, u32::from(b)).unwrap();
    }

    // Clear stale status, then kick the transaction (runs synchronously).
    bus.write_u32(I2C0_BASE + INT_CLR, 0xFFFF_FFFF).unwrap();
    bus.write_u32(I2C0_BASE + CTR, CTR_TRANS_START).unwrap();
    bus.read_u32(I2C0_BASE + INT_RAW).unwrap()
}

/// A data-stream write lands the exact payload in SSD1306 GDDRAM at page 0 /
/// column 0 (the panel's power-on horizontal-addressing origin), and the
/// address phase is ACKed (no NACK).
#[test]
fn s3_i2c_data_stream_renders_expected_framebuffer() {
    let mut bus = build_bus();

    // Blank at boot: firmware has not drawn anything.
    assert!(
        read_ssd1306(&bus).framebuffer().iter().all(|&b| b == 0),
        "SSD1306 GDDRAM must be blank before any transaction"
    );

    // Control byte 0x40 (data stream) + a deterministic pixel pattern.
    let pattern: [u8; 8] = [0xFF, 0x81, 0x42, 0x24, 0x18, 0xAA, 0x55, 0x3C];
    let mut payload = vec![CTRL_DATA_STREAM];
    payload.extend_from_slice(&pattern);

    let status = i2c_write_transaction(&mut bus, &payload);
    assert_ne!(
        status & TRANS_COMPLETE,
        0,
        "transaction must complete (TRANS_COMPLETE)"
    );
    assert_eq!(status & NACK, 0, "SSD1306 @ 0x3C must ACK its address");

    let oled = read_ssd1306(&bus);
    let fb = oled.framebuffer();
    assert_eq!(fb.len(), 1024, "128x64 GDDRAM is 1024 bytes");
    assert_eq!(
        &fb[0..8],
        &pattern,
        "data-stream bytes must land verbatim at page 0 / column 0"
    );
    assert!(
        fb[8..].iter().all(|&b| b == 0),
        "only the written columns may be non-zero"
    );
    assert_eq!(
        oled.ink_bytes(),
        pattern.iter().filter(|&&b| b != 0).count(),
        "ink accounting must match the non-zero payload bytes"
    );
}

/// A command-stream write reaches the panel's command path: 0xAF (display on)
/// flips `display_on()`, and a set-page / set-column pair retargets where a
/// following data write lands.
#[test]
fn s3_i2c_command_stream_controls_panel_state() {
    let mut bus = build_bus();
    assert!(
        !read_ssd1306(&bus).display_on(),
        "panel starts with the display off"
    );

    // Command stream: 0xAF = display ON.
    assert_ne!(
        i2c_write_transaction(&mut bus, &[CTRL_CMD_STREAM, 0xAF]) & TRANS_COMPLETE,
        0
    );
    assert!(
        read_ssd1306(&bus).display_on(),
        "0xAF command must turn the display on"
    );

    // Command stream: set page 2 (0xB2) and column 5 (lower 0x05, upper 0x10),
    // then a data write must land at page-2/column-5, not the origin.
    assert_ne!(
        i2c_write_transaction(&mut bus, &[CTRL_CMD_STREAM, 0xB2, 0x05, 0x10]) & TRANS_COMPLETE,
        0
    );
    assert_ne!(
        i2c_write_transaction(&mut bus, &[CTRL_DATA_STREAM, 0x7E]) & TRANS_COMPLETE,
        0
    );

    let oled = read_ssd1306(&bus);
    let fb = oled.framebuffer();
    let idx = 2 * 128 + 5; // page 2, column 5, page-major layout
    assert_eq!(
        fb[idx], 0x7E,
        "addressed data byte must land at page 2 / column 5"
    );
    assert_eq!(
        oled.ink_bytes(),
        1,
        "exactly one GDDRAM byte was written after re-addressing"
    );
}
