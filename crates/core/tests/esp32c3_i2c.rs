// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ESP32-C3 main I²C0 controller integration test.
//!
//! Drives the C3 I2C0 command-list engine (base 0x6001_3000) the way esp-hal
//! does — program a COMD list, push the address/pointer bytes into the TX FIFO,
//! kick TRANS_START — and reads a known register out of an attached TMP102
//! temperature sensor.
//!
//! Two layers are exercised:
//!   1. The peripheral model in isolation (`Esp32c3I2c` + `attach_slave`).
//!   2. The full C3 system bus assembled from `configs/chips/esp32c3.yaml`,
//!      proving I2C0 is wired at the right MMIO base with a reachable slave.

use std::path::PathBuf;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32c3::i2c::{
    Esp32c3I2c, I2C0_BASE, INT_TRANS_COMPLETE,
};
use labwired_core::peripherals::esp32s3::tmp102::Tmp102;
use labwired_core::Peripheral;

// COMD opcodes (ESP32-C3 TRM §17): 1=WRITE, 2=STOP, 3=READ, 4=END, 6=RSTART.
const CMD_WRITE: u8 = 1;
const CMD_STOP: u8 = 2;
const CMD_READ: u8 = 3;
const CMD_RSTART: u8 = 6;

// Register offsets.
const REG_CTR: u64 = 0x04;
const REG_DATA: u64 = 0x1C;
const REG_INT_RAW: u64 = 0x20;
const REG_CMD0: u64 = 0x58;
const CTR_TRANS_START_BIT: u32 = 1 << 5;

fn cmd(opcode: u8, byte_num: u8) -> u32 {
    ((opcode as u32 & 0x7) << 11) | (byte_num as u32)
}

/// Program the canonical TMP102 pointer-then-read sequence into whatever
/// `write`/`read` sink is given (a bare peripheral or an MMIO base on a bus).
fn tmp102_read_sequence<W: FnMut(u64, u32)>(mut w: W) {
    // RSTART; WRITE 2 (addr+W, pointer=0); RSTART; WRITE 1 (addr+R); READ 2; STOP.
    w(REG_CMD0, cmd(CMD_RSTART, 0));
    w(REG_CMD0 + 4, cmd(CMD_WRITE, 2));
    w(REG_CMD0 + 8, cmd(CMD_RSTART, 0));
    w(REG_CMD0 + 12, cmd(CMD_WRITE, 1));
    w(REG_CMD0 + 16, cmd(CMD_READ, 2));
    w(REG_CMD0 + 20, cmd(CMD_STOP, 0));

    // TX bytes: addr+W (0x48<<1), pointer 0 (temperature reg), addr+R.
    w(REG_DATA, 0x90);
    w(REG_DATA, 0x00);
    w(REG_DATA, 0x91);

    // Kick the command list.
    w(REG_CTR, CTR_TRANS_START_BIT);
}

#[test]
fn esp32c3_i2c0_reads_attached_tmp102() {
    let mut i2c = Esp32c3I2c::new();
    i2c.attach_slave(Box::new(Tmp102::new()));

    tmp102_read_sequence(|off, val| i2c.write_u32(off, val).unwrap());

    // TMP102 initial temperature 25.0 °C -> 0x1900 left-justified: MSB 0x19, LSB 0x00.
    assert_eq!(i2c.read_u32(REG_DATA).unwrap(), 0x19, "TMP102 temp MSB");
    assert_eq!(i2c.read_u32(REG_DATA).unwrap(), 0x00, "TMP102 temp LSB");
    assert_eq!(
        i2c.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
        INT_TRANS_COMPLETE,
        "STOP must raise TRANS_COMPLETE"
    );
}

#[test]
fn esp32c3_system_bus_wires_i2c0_with_reachable_slave() {
    // Build the whole C3 system from the shipped chip config and drive I2C0
    // over the MMIO bus — this exercises the config-driven wiring path
    // (`esp32c3_i2c` factory arm + chip YAML) end to end.
    let chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("configs/chips/esp32c3.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load esp32c3 chip descriptor");

    let manifest = SystemManifest {
        walk_deleted: false,
        schema_version: "1.0".to_string(),
        name: "esp32c3-i2c-test".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        peripherals: vec![],
        memory_overrides: Default::default(),
    };

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build esp32c3 system bus");

    let base = I2C0_BASE as u64;
    // I2C0 must be mapped at the C3 base and read its silicon reset values.
    assert_eq!(
        bus.read_u32(base + REG_CTR).unwrap(),
        0x0000_020B,
        "I2C0 CTR reset value at the mapped base"
    );

    // Drive the TMP102 read sequence through the bus and read it back.
    tmp102_read_sequence(|off, val| bus.write_u32(base + off, val).unwrap());
    assert_eq!(bus.read_u32(base + REG_DATA).unwrap(), 0x19, "temp MSB via bus");
    assert_eq!(bus.read_u32(base + REG_DATA).unwrap(), 0x00, "temp LSB via bus");
    assert_eq!(
        bus.read_u32(base + REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
        INT_TRANS_COMPLETE,
        "TRANS_COMPLETE via bus"
    );
}
