// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 DW_apb_i2c master driving an attached I²C slave.
//!
//! Two levels of coverage:
//!  1. `dw_apb_i2c_reads_tmp102_register` — drives the DW_apb_i2c registers
//!     directly against a TMP102 attached to the peripheral.
//!  2. `rp2040_system_i2c0_reaches_attached_slave` — builds the full RP2040
//!     bus from the shipped chip/system configs (I2C0 auto-attaches a TMP102)
//!     and reaches the slave over the MMIO bus at I2C0's base 0x4004_4000.

use labwired_core::peripherals::esp32s3::tmp102::Tmp102;
use labwired_core::peripherals::rp2040::i2c::{Rp2040I2c, I2C0_BASE};
use labwired_core::Peripheral;

// DW_apb_i2c register offsets (RP2040 §4.3.16).
const IC_TAR: u64 = 0x04;
const IC_DATA_CMD: u64 = 0x10;
const IC_RAW_INTR_STAT: u64 = 0x34;
const IC_ENABLE: u64 = 0x6c;
const IC_STATUS: u64 = 0x70;
const IC_RXFLR: u64 = 0x78;

// IC_DATA_CMD command bits.
const CMD_READ: u32 = 1 << 8;
const CMD_STOP: u32 = 1 << 9;
const CMD_RESTART: u32 = 1 << 10;

// IC_STATUS / IC_RAW_INTR_STAT bits used by the assertions.
const STATUS_RFNE: u32 = 1 << 3;
const INTR_STOP_DET: u32 = 1 << 9;

/// Drive the raw DW_apb_i2c registers to read the TMP102 temperature register
/// (pointer 0): expect 0x19, 0x00 (25.0 °C, left-justified 12-bit value).
#[test]
fn dw_apb_i2c_reads_tmp102_register() {
    let mut i2c = Rp2040I2c::new();
    i2c.attach_slave(Box::new(Tmp102::new()));

    i2c.write_u32(IC_ENABLE, 1).unwrap();
    i2c.write_u32(IC_TAR, 0x48).unwrap();

    // Write pointer 0x00 (no STOP → keep the bus for the repeated start),
    // then read 2 bytes: first with RESTART, last with STOP.
    i2c.write_u32(IC_DATA_CMD, 0x00).unwrap();
    i2c.write_u32(IC_DATA_CMD, CMD_RESTART | CMD_READ).unwrap();
    i2c.write_u32(IC_DATA_CMD, CMD_STOP | CMD_READ).unwrap();

    assert_eq!(i2c.read_u32(IC_RXFLR).unwrap(), 2, "two bytes queued in RX FIFO");
    assert_ne!(
        i2c.read_u32(IC_STATUS).unwrap() & STATUS_RFNE,
        0,
        "RFNE must be set with data pending"
    );

    let msb = i2c.read_u32(IC_DATA_CMD).unwrap() & 0xFF;
    let lsb = i2c.read_u32(IC_DATA_CMD).unwrap() & 0xFF;
    assert_eq!(msb, 0x19, "TMP102 temperature MSB");
    assert_eq!(lsb, 0x00, "TMP102 temperature LSB");

    assert_ne!(
        i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET,
        0,
        "STOP must raise STOP_DET"
    );
}

/// Build the whole RP2040 bus from the shipped configs and reach the TMP102
/// that I2C0 auto-attaches, going through the memory-mapped bus at 0x4004_4000.
#[test]
fn rp2040_system_i2c0_reaches_attached_slave() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use std::path::PathBuf;

    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/rp2040.yaml");
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/rp2040-pico.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).expect("load rp2040 chip config");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load rp2040-pico manifest");
    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build RP2040 bus");

    let base = I2C0_BASE as u64;
    bus.write_u32(base + IC_ENABLE, 1).unwrap();
    bus.write_u32(base + IC_TAR, 0x48).unwrap();
    bus.write_u32(base + IC_DATA_CMD, 0x00).unwrap();
    bus.write_u32(base + IC_DATA_CMD, CMD_RESTART | CMD_READ).unwrap();
    bus.write_u32(base + IC_DATA_CMD, CMD_STOP | CMD_READ).unwrap();

    let msb = bus.read_u32(base + IC_DATA_CMD).unwrap() & 0xFF;
    let lsb = bus.read_u32(base + IC_DATA_CMD).unwrap() & 0xFF;
    assert_eq!(msb, 0x19, "TMP102 MSB via I2C0 MMIO");
    assert_eq!(lsb, 0x00, "TMP102 LSB via I2C0 MMIO");
}
