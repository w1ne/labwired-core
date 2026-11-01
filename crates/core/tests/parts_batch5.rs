// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Batch 5: one smoke check per part, and the negative control that keeps
//! each check honest.**
//!
//! Same contract as `everyday_sensor_set.rs`: every part is a
//! `configs/devices/*.yaml` descriptor with no Rust model, and each gets a
//! `check_*` function that talks to it the way its datasheet and its common
//! drivers do and asserts REAL register values. The negative controls run the
//! very same functions against stand-ins that answer every byte with `0x00`,
//! and with `0xFF`; both must FAIL every check. A smoke test a zero-answering
//! part could pass proves nothing.
//!
//! The SPI ADCs (MCP3008, ADS1256) are the first users of
//! `behavior.spi.miso_var`: a message part whose reply byte is computed by the
//! rules for the byte being clocked.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;

mod common;
use common::transcript::{read_reg, run_i2c, run_spi, spi_xfer, write_reg, Step};

// ─── plumbing ──────────────────────────────────────────────────────────────

fn dev(device_type: &str, addr: u8) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    GenericI2cDevice::from_yaml(yaml, addr).unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

fn spi_dev(device_type: &str) -> GenericSpiDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    GenericSpiDevice::from_yaml(yaml, "PA4").unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

fn rd(d: &mut dyn I2cDevice, ptr: u8, n: usize) -> Vec<u8> {
    run_i2c(d, &read_reg(ptr, n)).bytes
}

fn wr(d: &mut dyn I2cDevice, ptr: u8, bytes: &[u8]) {
    run_i2c(d, &write_reg(ptr, bytes));
}

fn send(d: &mut dyn I2cDevice, bytes: &[u8]) {
    let mut s = vec![Step::Start];
    s.extend(bytes.iter().map(|&b| Step::Write(b)));
    s.push(Step::Stop);
    run_i2c(d, &s);
}

fn le16(b: &[u8]) -> u16 {
    u16::from(b[1]) << 8 | u16::from(b[0])
}

/// One CS-framed SPI transaction; every MISO byte.
fn xfer(d: &mut dyn SpiDevice, mosi: &[u8]) -> Vec<u8> {
    run_spi(d, &spi_xfer(mosi)).bytes
}

fn spi_advance(d: &mut dyn SpiDevice, us: u64) {
    run_spi(d, &[Step::AdvanceUs(us)]);
}

macro_rules! ensure_eq {
    ($got:expr, $want:expr, $($what:tt)+) => {{
        let (g, w) = ($got, $want);
        if g != w {
            return Err(format!("{}: got {:02X?}, want {:02X?}", format!($($what)+), g, w));
        }
    }};
}

// ─── I²C checks (default stimulus) ─────────────────────────────────────────

/// BMI160 (BST-BMI160-DS000-09 §2.11): CHIP_ID 0xD1; both sensors power up
/// suspended (PMU_STATUS 0x00, ACC_Z 0x0000, TEMPERATURE 0x8000 invalid); CMD
/// 0x11 / 0x15 bring them to normal mode, after which ACC_Z is 1 g at ±2 g
/// (16384 = 0x4000) and the temperature is (25 − 23)·512 = 0x0400.
fn check_bmi160(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x00, 1), vec![0xD1u8], "BMI160 CHIP_ID");
    ensure_eq!(
        rd(d, 0x03, 1),
        vec![0x00u8],
        "BMI160 PMU_STATUS at power-on"
    );
    ensure_eq!(
        rd(d, 0x16, 2),
        vec![0x00u8, 0x00],
        "BMI160 ACC_Z while suspended"
    );
    ensure_eq!(
        rd(d, 0x20, 2),
        vec![0x00u8, 0x80],
        "BMI160 TEMPERATURE invalid while gyro suspended"
    );
    ensure_eq!(
        rd(d, 0x40, 4),
        vec![0x28u8, 0x03, 0x28, 0x00],
        "BMI160 ACC_CONF..GYR_RANGE reset"
    );
    wr(d, 0x7E, &[0x11]);
    wr(d, 0x7E, &[0x15]);
    ensure_eq!(
        rd(d, 0x03, 1),
        vec![0x14u8],
        "BMI160 PMU_STATUS: acc normal, gyr normal"
    );
    ensure_eq!(
        rd(d, 0x1B, 1),
        vec![0xC0u8],
        "BMI160 STATUS drdy_acc | drdy_gyr"
    );
    ensure_eq!(
        rd(d, 0x12, 6),
        vec![0x00u8, 0x00, 0x00, 0x00, 0x00, 0x40],
        "BMI160 ACC X/Y/Z at (0, 0, 1 g)"
    );
    ensure_eq!(
        rd(d, 0x20, 2),
        vec![0x00u8, 0x04],
        "BMI160 TEMPERATURE at 25 °C"
    );
    ensure_eq!(
        rd(d, 0x7E, 1),
        vec![0x00u8],
        "BMI160 CMD reads back 0 (write-only)"
    );
    Ok(())
}

/// The Bosch BMM150 integer compensation (Zephyr `bmm150_compensate_xy/_z`,
/// itself the Bosch Sensor API's), over the trim block the part serves.
/// Returns (x, y, z) in 1/16 µT.
fn bmm150_compensate(trim: &[u8], data: &[u8]) -> (i32, i32, i32) {
    let i8_at = |a: usize| trim[a - 0x5D] as i8 as i32;
    let i16_at = |a: usize| i16::from_le_bytes([trim[a - 0x5D], trim[a - 0x5D + 1]]) as i32;
    let u16_at = |a: usize| u16::from_le_bytes([trim[a - 0x5D], trim[a - 0x5D + 1]]) as i32;
    let (x1, y1, x2, y2) = (i8_at(0x5D), i8_at(0x5E), i8_at(0x64), i8_at(0x65));
    let (z4, z2, z1, xyz1, z3) = (
        i16_at(0x62),
        i16_at(0x68),
        u16_at(0x6A),
        u16_at(0x6C),
        i16_at(0x6E),
    );
    let (xy2, xy1) = (i8_at(0x70), trim[0x71 - 0x5D] as i32);
    let raw_x = (i16::from_le_bytes([data[0], data[1]]) >> 3) as i32;
    let raw_y = (i16::from_le_bytes([data[2], data[3]]) >> 3) as i32;
    let raw_z = (i16::from_le_bytes([data[4], data[5]]) >> 1) as i32;
    let rhall = (u16::from_le_bytes([data[6], data[7]]) >> 2) as i32;
    let xy = |raw: i32, t1: i32, t2: i32| {
        let prevalue = ((xyz1 << 14) / rhall) as u16;
        let val = prevalue.wrapping_sub(0x4000) as i16 as i32;
        let temp1 = xy2 * ((val * val) >> 7);
        let temp2 = val * (xy1 << 7);
        let temp3 = ((((temp1 + temp2) >> 9) + 0x100000) * (t2 + 0xA0)) >> 12;
        (((raw * temp3) >> 13) as i16 as i32) + (t1 << 3)
    };
    let temp1 = (raw_z - z4) << 15;
    let temp2 = (z3 * (rhall - xyz1)) >> 2;
    let temp3 = ((z1 * (rhall << 1)) + (1 << 15)) >> 16;
    (
        xy(raw_x, x1, x2),
        xy(raw_y, y1, y2),
        (temp1 - temp2) / (z2 + temp3),
    )
}

/// BMM150 (BST-BMM150-DS001-05 §4.2/§5.3): the Chip ID reads 0x00 in Suspend
/// and 0x32 once the Power Control bit is set; OpMode resets to 0x06 (sleep);
/// the Bosch compensation over the served trim recovers the default field
/// (20, −5, 40 µT → 320, −80, 640 in 1/16 µT).
fn check_bmm150(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(
        rd(d, 0x40, 1),
        vec![0x00u8],
        "BMM150 Chip ID in suspend (I²C reads 0x00)"
    );
    wr(d, 0x4B, &[0x01]);
    ensure_eq!(rd(d, 0x4B, 1), vec![0x01u8], "BMM150 power control");
    ensure_eq!(
        rd(d, 0x40, 1),
        vec![0x32u8],
        "BMM150 Chip ID after power-up"
    );
    ensure_eq!(
        rd(d, 0x4C, 3),
        vec![0x06u8, 0x3F, 0x07],
        "BMM150 OpMode / INT_EN / INT_CONF defaults"
    );
    let trim = rd(d, 0x5D, 0x71 - 0x5D + 1);
    ensure_eq!(le16(&trim[0x6C - 0x5D..]), 6500u16, "BMM150 dig_xyz1");
    let data = rd(d, 0x42, 8);
    ensure_eq!(data[6] & 0x01, 0x01u8, "BMM150 Data Ready bit");
    ensure_eq!(
        bmm150_compensate(&trim, &data),
        (320, -80, 640),
        "BMM150 compensated field (1/16 µT)"
    );
    Ok(())
}

/// VEML6075 (Vishay 84304 Table 1): ID 0x0026; UV_CONF 0x0000; UVA at
/// 100 µW/cm² = 93 counts (0.93 counts per µW/cm² at 50 ms); UVB = 210
/// (2.1); the compensation channels at their default counts.
fn check_veml6075(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x0C, 2), vec![0x26u8, 0x00], "VEML6075 ID");
    ensure_eq!(
        rd(d, 0x00, 2),
        vec![0x00u8, 0x00],
        "VEML6075 UV_CONF default"
    );
    ensure_eq!(
        le16(&rd(d, 0x07, 2)),
        93u16,
        "VEML6075 UVA at 100 µW/cm², 50 ms"
    );
    ensure_eq!(
        le16(&rd(d, 0x09, 2)),
        210u16,
        "VEML6075 UVB at 100 µW/cm², 50 ms"
    );
    ensure_eq!(le16(&rd(d, 0x0A, 2)), 20u16, "VEML6075 UVCOMP1");
    ensure_eq!(le16(&rd(d, 0x0B, 2)), 10u16, "VEML6075 UVCOMP2");
    Ok(())
}

/// HT16K33 (Holtek V1.10 Command Summary): oscillator on (21h), display on
/// (81h), full brightness (EFh), then 16 display-RAM bytes from pointer 00h,
/// read back through the same pointer; key RAM (40h..45h) reads 0x00.
fn check_ht16k33(d: &mut dyn I2cDevice) -> Result<(), String> {
    send(d, &[0x21]);
    send(d, &[0x81]);
    send(d, &[0xEF]);
    let pattern: Vec<u8> = (0..16u8)
        .map(|i| 0x11u8.wrapping_mul(i + 1) ^ 0x5A)
        .collect();
    let mut frame = vec![0x00u8];
    frame.extend(&pattern);
    send(d, &frame);
    ensure_eq!(
        rd(d, 0x00, 16),
        pattern.clone(),
        "HT16K33 display RAM 00h..0Fh"
    );
    ensure_eq!(
        rd(d, 0x05, 2),
        pattern[5..7].to_vec(),
        "HT16K33 display RAM from pointer 05h"
    );
    ensure_eq!(
        rd(d, 0x40, 6),
        vec![0x00u8; 6],
        "HT16K33 key RAM (no key pressed)"
    );
    Ok(())
}

type Check = fn(&mut dyn I2cDevice) -> Result<(), String>;

const PARTS: &[(&str, u8, Check)] = &[
    ("bmi160", 0x68, check_bmi160),
    ("bmm150", 0x10, check_bmm150),
    ("veml6075", 0x10, check_veml6075),
    ("ht16k33", 0x70, check_ht16k33),
];

// ─── SPI checks (default stimulus) ─────────────────────────────────────────

/// One SX127x register access (§4.3): address byte (bit 7 = write), one data
/// byte. Returns the MISO byte of the data phase.
fn sx(d: &mut dyn SpiDevice, addr: u8, data: u8) -> u8 {
    xfer(d, &[addr, data])[1]
}

/// SX1276 (Rev. 4 Table 41 / §6.4): RegVersion 0x12, RegOpMode 0x01, the LoRa
/// page's RegModemConfig1/2 (0x72/0x70), RegSyncWord 0x12 and
/// RegFifoTxBaseAddr 0x80; in LoRa mode RegRssiValue = −100 + 157 = 57.
fn check_sx1276(d: &mut dyn SpiDevice) -> Result<(), String> {
    ensure_eq!(sx(d, 0x42, 0), 0x12u8, "SX1276 RegVersion");
    ensure_eq!(sx(d, 0x01, 0), 0x01u8, "SX1276 RegOpMode POR");
    ensure_eq!(
        [
            sx(d, 0x1D, 0),
            sx(d, 0x1E, 0),
            sx(d, 0x39, 0),
            sx(d, 0x0E, 0)
        ],
        [0x72u8, 0x70, 0x12, 0x80],
        "SX1276 RegModemConfig1/2, RegSyncWord, RegFifoTxBaseAddr"
    );
    sx(d, 0x81, 0x80); // LoRa, sleep
    sx(d, 0x81, 0x81); // LoRa, standby
    ensure_eq!(
        sx(d, 0x01, 0),
        0x81u8,
        "SX1276 RegOpMode after LoRa standby"
    );
    ensure_eq!(sx(d, 0x1B, 0), 57u8, "SX1276 RegRssiValue at −100 dBm (HF)");
    ensure_eq!(sx(d, 0x19, 0), 38u8, "SX1276 RegPktSnrValue at 9.5 dB");
    Ok(())
}

/// MCP3008 (DS21295D Figure 6-1, Equation 4-2): 0x01, SGL·D2D1D0 << 4, 0x00.
/// Code = (b1 & 3) << 8 | b2. CH0 1650 mV → 512, CH5 1000 mV → 310,
/// differential CH0−CH1 (825 mV) → 256, all at VREF 3300 mV.
fn mcp3008_read(d: &mut dyn SpiDevice, cfg: u8) -> u16 {
    let r = xfer(d, &[0x01, cfg << 4, 0x00]);
    u16::from(r[1] & 0x03) << 8 | u16::from(r[2])
}

fn check_mcp3008(d: &mut dyn SpiDevice) -> Result<(), String> {
    let r = xfer(d, &[0x01, 0x80, 0x00]);
    ensure_eq!(r[1] & 0x04, 0x00u8, "MCP3008 null bit");
    ensure_eq!(
        mcp3008_read(d, 0x8),
        512u16,
        "MCP3008 CH0 single-ended at 1650 mV"
    );
    ensure_eq!(
        mcp3008_read(d, 0xD),
        310u16,
        "MCP3008 CH5 single-ended at 1000 mV"
    );
    ensure_eq!(mcp3008_read(d, 0x0), 256u16, "MCP3008 CH0−CH1 differential");
    ensure_eq!(
        mcp3008_read(d, 0x1),
        0u16,
        "MCP3008 CH1−CH0 (negative) clamps to 0"
    );
    Ok(())
}

/// ADS1256 (SBAS288K Tables 23/24/16): after t18, RREG 00h ×5 reads STATUS
/// 0x30 (ID 3, DRDY low), MUX 01h, ADCON 20h, DRATE F0h, IO E0h; RDATA on the
/// default AIN0−AIN1 = 1.25 V at PGA 1, VREF 2.5 V → 0x1FFFFF.
fn check_ads1256(d: &mut dyn SpiDevice) -> Result<(), String> {
    spi_advance(d, 250);
    let r = xfer(d, &[0x10, 0x04, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    ensure_eq!(
        r[2..].to_vec(),
        vec![0x30u8, 0x01, 0x20, 0xF0, 0xE0],
        "ADS1256 RREG STATUS..IO"
    );
    let r = xfer(d, &[0x01, 0xFF, 0xFF, 0xFF]);
    ensure_eq!(
        r[1..].to_vec(),
        vec![0x1Fu8, 0xFF, 0xFF],
        "ADS1256 RDATA AIN0−AIN1 at 1.25 V"
    );
    Ok(())
}

/// SX1262 (DS.SX1261-2 Rev 1.2): GetStatus (Table 13-77) answers 0x20 —
/// STBY_RC, no command status — at power-on; ReadRegister (Table 13-25) of the
/// LoRa sync word 0x0740 returns Table 12-1's 0x14 0x24 after three status
/// bytes; SetStandby(STDBY_XOSC) moves ChipMode to 0x3; SetPacketType(LoRa)
/// reads back through GetPacketType.
fn check_sx1262(d: &mut dyn SpiDevice) -> Result<(), String> {
    ensure_eq!(
        xfer(d, &[0xC0, 0x00]),
        vec![0x20u8, 0x20],
        "SX1262 GetStatus at power-on"
    );
    ensure_eq!(
        xfer(d, &[0x1D, 0x07, 0x40, 0x00, 0x00, 0x00]),
        vec![0x20u8, 0x20, 0x20, 0x20, 0x14, 0x24],
        "SX1262 ReadRegister LoRa sync word"
    );
    ensure_eq!(
        xfer(d, &[0x1D, 0x08, 0xE7, 0x00, 0x00])[4],
        0x38u8,
        "SX1262 OCP (140 mA)"
    );
    xfer(d, &[0x80, 0x01]);
    ensure_eq!(
        xfer(d, &[0xC0, 0x00])[1],
        0x30u8,
        "SX1262 status after SetStandby(STDBY_XOSC)"
    );
    ensure_eq!(
        xfer(d, &[0x11, 0x00, 0x00])[2],
        0x00u8,
        "SX1262 packet type at power-on (GFSK)"
    );
    xfer(d, &[0x8A, 0x01]);
    ensure_eq!(
        xfer(d, &[0x11, 0x00, 0x00]),
        vec![0x30u8, 0x30, 0x01],
        "SX1262 GetPacketType after LoRa"
    );
    Ok(())
}

type SpiCheck = fn(&mut dyn SpiDevice) -> Result<(), String>;

const SPI_PARTS: &[(&str, SpiCheck)] = &[
    ("sx1276", check_sx1276),
    ("sx1262", check_sx1262),
    ("mcp3008", check_mcp3008),
    ("ads1256", check_ads1256),
];

// ─── smoke + negative control ──────────────────────────────────────────────

#[test]
fn every_part_passes_its_smoke_check() {
    let mut failures = Vec::new();
    for &(ty, addr, check) in PARTS {
        let mut d = dev(ty, addr);
        assert_eq!(d.address(), addr, "{ty}: default address");
        match check(&mut d) {
            Ok(()) => println!("smoke  {ty:<9} @0x{addr:02X}: PASS"),
            Err(e) => failures.push(format!("{ty}: {e}")),
        }
    }
    for &(ty, check) in SPI_PARTS {
        let mut d = spi_dev(ty);
        match check(&mut d) {
            Ok(()) => println!("smoke  {ty:<9} (SPI): PASS"),
            Err(e) => failures.push(format!("{ty}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "smoke failures:\n{}",
        failures.join("\n")
    );
}

/// A stand-in I²C part that ACKs everything and answers every read with one byte.
struct Silent {
    byte: u8,
}

impl I2cDevice for Silent {
    fn address(&self) -> u8 {
        0
    }
    fn read(&mut self) -> u8 {
        self.byte
    }
    fn write(&mut self, _data: u8) {}
}

/// A stand-in SPI part that answers every byte with one value.
struct SilentSpi {
    byte: u8,
}

impl SpiDevice for SilentSpi {
    fn transfer(&mut self, _mosi: u8) -> u8 {
        self.byte
    }
    fn cs_pin(&self) -> &str {
        "PA4"
    }
}

#[test]
fn negative_control_a_silent_part_fails_every_check() {
    for byte in [0x00u8, 0xFF] {
        for &(ty, _, check) in PARTS {
            match check(&mut Silent { byte }) {
                Err(e) => {
                    println!("negctl {ty:<9} answering 0x{byte:02X}: FAILS as required ({e})")
                }
                Ok(()) => panic!(
                    "{ty}: a part answering only 0x{byte:02X} PASSED the smoke check — the \
                     check proves nothing"
                ),
            }
        }
        for &(ty, check) in SPI_PARTS {
            match check(&mut SilentSpi { byte }) {
                Err(e) => {
                    println!("negctl {ty:<9} answering 0x{byte:02X}: FAILS as required ({e})")
                }
                Ok(()) => panic!(
                    "{ty}: a SPI part answering only 0x{byte:02X} PASSED the smoke check — the \
                     check proves nothing"
                ),
            }
        }
    }
}

// ─── per-part behaviour beyond the default stimulus ────────────────────────

#[test]
fn bmi160_scales_with_range_and_softreset_restores_defaults() {
    let mut d = dev("bmi160", 0x68);
    wr(&mut d, 0x7E, &[0x11]);
    wr(&mut d, 0x7E, &[0x15]);
    // ±8 g: 4096 LSB/g; −0.5 g on X → −2048.
    wr(&mut d, 0x41, &[0x08]);
    run_i2c(&mut d, &[Step::Input("accel_x", -0.5)]);
    assert_eq!(le16(&rd(&mut d, 0x12, 2)) as i16, -2048);
    // ±250 °/s: 131.2 LSB/°/s; 100 °/s on Z → 13120.
    wr(&mut d, 0x43, &[0x03]);
    run_i2c(&mut d, &[Step::Input("gyro_z", 100.0)]);
    assert_eq!(le16(&rd(&mut d, 0x10, 2)) as i16, 13120);
    // 40 °C → (40 − 23)·512 = 8704.
    run_i2c(&mut d, &[Step::Input("temperature", 40.0)]);
    assert_eq!(le16(&rd(&mut d, 0x20, 2)), 8704);
    // Reserved high bits of ACC_RANGE stay 0.
    wr(&mut d, 0x41, &[0xF5]);
    assert_eq!(rd(&mut d, 0x41, 1), vec![0x05]);
    // Gyro back to suspend: temperature invalid again, gyro data 0.
    wr(&mut d, 0x7E, &[0x14]);
    assert_eq!(rd(&mut d, 0x20, 2), vec![0x00, 0x80]);
    assert_eq!(rd(&mut d, 0x10, 2), vec![0x00, 0x00]);
    // Softreset: everything back to Figure 20.
    wr(&mut d, 0x7E, &[0xB6]);
    assert_eq!(rd(&mut d, 0x03, 1), vec![0x00]);
    assert_eq!(rd(&mut d, 0x40, 4), vec![0x28, 0x03, 0x28, 0x00]);
    assert_eq!(rd(&mut d, 0x00, 1), vec![0xD1]);
}

#[test]
fn bmm150_tracks_the_field_and_suspend_hides_every_register() {
    let mut d = dev("bmm150", 0x10);
    // Suspend: the whole map reads 0x00 except POWER_CTRL.
    assert_eq!(rd(&mut d, 0x42, 8), vec![0u8; 8]);
    assert_eq!(rd(&mut d, 0x6C, 2), vec![0, 0]);
    wr(&mut d, 0x4B, &[0x01]);
    let trim = rd(&mut d, 0x5D, 0x71 - 0x5D + 1);
    // Fields on the LSB grid (0.3125 µT X/Y, 0.5 µT Z) come back exactly.
    for (x, y, z) in [(0.0, 0.0, 0.0), (45.0, -30.0, -60.5), (-100.0, 12.5, 250.0)] {
        run_i2c(
            &mut d,
            &[
                Step::Input("mag_x", x),
                Step::Input("mag_y", y),
                Step::Input("mag_z", z),
            ],
        );
        let got = bmm150_compensate(&trim, &rd(&mut d, 0x42, 8));
        let want = (
            (x * 16.0f64).round() as i32,
            (y * 16.0f64).round() as i32,
            (z * 16.0f64).round() as i32,
        );
        assert_eq!(got, want, "field ({x}, {y}, {z}) µT");
    }
    // Soft reset: config back to defaults, power bit kept, triggers read 0.
    wr(&mut d, 0x4C, &[0x2A]);
    wr(&mut d, 0x51, &[0x04]);
    wr(&mut d, 0x4B, &[0x83]);
    assert_eq!(rd(&mut d, 0x4B, 1), vec![0x01]);
    assert_eq!(rd(&mut d, 0x4C, 1), vec![0x06]);
    assert_eq!(rd(&mut d, 0x51, 1), vec![0x00]);
    // Power bit cleared: back to suspend, Chip ID 0x00 again.
    wr(&mut d, 0x4B, &[0x00]);
    assert_eq!(rd(&mut d, 0x40, 1), vec![0x00]);
}

#[test]
fn veml6075_counts_follow_integration_time_and_stimulus() {
    let mut d = dev("veml6075", 0x10);
    // UV_IT = 100 ms doubles every count.
    wr(&mut d, 0x00, &[0x10, 0x00]);
    assert_eq!(le16(&rd(&mut d, 0x07, 2)), 186);
    assert_eq!(le16(&rd(&mut d, 0x09, 2)), 420);
    // 800 ms (×16) at 250 µW/cm² UVA → 0.93·250·16 = 3720.
    wr(&mut d, 0x00, &[0x40, 0x00]);
    run_i2c(&mut d, &[Step::Input("uva", 250.0)]);
    assert_eq!(le16(&rd(&mut d, 0x07, 2)), 3720);
    // The reserved high byte and bit 7 cannot be set; UV_TRIG self-clears.
    wr(&mut d, 0x00, &[0xFF, 0xFF]);
    assert_eq!(rd(&mut d, 0x00, 2), vec![0x7B, 0x00]);
    // ID is read-only.
    wr(&mut d, 0x0C, &[0x00, 0x00]);
    assert_eq!(rd(&mut d, 0x0C, 2), vec![0x26, 0x00]);
}

#[test]
fn ht16k33_decodes_its_setup_commands() {
    let mut d = dev("ht16k33", 0x70);
    let var = |d: &GenericI2cDevice, n: &str| d.rule_machine().expect("rules").var(n);
    assert_eq!(
        (var(&d, "osc_on"), var(&d, "display_on"), var(&d, "dimming")),
        (0, 0, 15)
    );
    send(&mut d, &[0x21]);
    send(&mut d, &[0x85]); // display on, B1:B0 = 10 (1 Hz blink)
    send(&mut d, &[0xE7]);
    assert_eq!(var(&d, "osc_on"), 1);
    assert_eq!(var(&d, "display_on"), 1);
    assert_eq!(var(&d, "blink"), 2);
    assert_eq!(var(&d, "dimming"), 7);
    send(&mut d, &[0x80]);
    assert_eq!(var(&d, "display_on"), 0);
    // A display-RAM write is not a setup command, whatever its data bytes.
    send(&mut d, &[0x02, 0x81, 0xE0]);
    assert_eq!(var(&d, "display_on"), 0);
    assert_eq!(var(&d, "dimming"), 7);
    assert_eq!(rd(&mut d, 0x02, 2), vec![0x81, 0xE0]);
}

#[test]
fn sx1276_tx_raises_txdone_and_irq_flags_clear_on_one() {
    let mut d = spi_dev("sx1276");
    // FSK page (LongRangeMode clear): the measurement registers read 0.
    assert_eq!(sx(&mut d, 0x1B, 0), 0x00);
    sx(&mut d, 0x81, 0x80);
    // Low-frequency port: RSSI = −164 + Rssi.
    sx(&mut d, 0x81, 0x89);
    run_spi(&mut d, &[Step::Input("rssi_dbm", -120.0)]);
    assert_eq!(sx(&mut d, 0x1B, 0), 44);
    sx(&mut d, 0x81, 0x81);
    assert_eq!(sx(&mut d, 0x1B, 0), 37);
    run_spi(&mut d, &[Step::Input("snr_db", -7.25)]);
    assert_eq!(sx(&mut d, 0x19, 0), (-29i8) as u8);
    // A cleared flag register stays clear when the driver writes 0xFF.
    sx(&mut d, 0x92, 0xFF);
    assert_eq!(sx(&mut d, 0x12, 0), 0x00);
    // TX: TxDone, and the radio is back in STDBY.
    sx(&mut d, 0x81, 0x83);
    assert_eq!(sx(&mut d, 0x12, 0), 0x08);
    assert_eq!(sx(&mut d, 0x01, 0), 0x81);
    sx(&mut d, 0x92, 0x08);
    assert_eq!(sx(&mut d, 0x12, 0), 0x00);
    // 915 MHz written and read back; RegVersion refuses a write.
    for (a, v) in [(0x06u8, 0xE4u8), (0x07, 0xC0), (0x08, 0x00)] {
        sx(&mut d, 0x80 | a, v);
    }
    assert_eq!(
        [
            sx(&mut d, 0x06, 0),
            sx(&mut d, 0x07, 0),
            sx(&mut d, 0x08, 0)
        ],
        [0xE4, 0xC0, 0x00]
    );
    sx(&mut d, 0xC2, 0x00);
    assert_eq!(sx(&mut d, 0x42, 0), 0x12);
}

#[test]
fn mcp3008_tracks_stimulus_vref_and_lsb_first_repeat() {
    let mut d = spi_dev("mcp3008");
    run_spi(
        &mut d,
        &[
            Step::Input("ch7_mv", 2345.0),
            Step::Input("vref_mv", 5000.0),
        ],
    );
    // 2345 · 1024 / 5000 = 480.256 → 480.
    assert_eq!(mcp3008_read(&mut d, 0xF), 480);
    // Full scale clamps to 1023.
    run_spi(&mut d, &[Step::Input("ch7_mv", 5400.0)]);
    assert_eq!(mcp3008_read(&mut d, 0xF), 1023);
    // Five bytes: the LSB-first repeat (B1..B8, then B9) follows B0.
    run_spi(&mut d, &[Step::Input("ch7_mv", 3051.0)]); // 624 = 0b10_0111_0000
    let r = xfer(&mut d, &[0x01, 0xF0, 0x00, 0x00, 0x00, 0x00]);
    let code = u16::from(r[1] & 3) << 8 | u16::from(r[2]);
    assert_eq!(code, 624);
    let mut lsb_first = 0u16;
    for i in 0..8 {
        lsb_first |= u16::from((r[3] >> (7 - i)) & 1) << (i + 1);
    }
    lsb_first |= u16::from(r[4] >> 7) << 9;
    assert_eq!(lsb_first, code & !1, "B1..B9 repeated LSB first");
    assert_eq!(r[4] & 0x7F, 0, "zeros after B9");
    assert_eq!(r[5], 0, "zeros indefinitely");
    // No start bit in byte 0: nothing is converted.
    assert_eq!(xfer(&mut d, &[0x00, 0xF0, 0x00]), vec![0, 0, 0]);
}

#[test]
fn ads1256_wreg_mux_pga_and_drdy_follow_the_datasheet() {
    let mut d = spi_dev("ads1256");
    // DRDY is high (STATUS bit 0 set) until the t18 settling time.
    assert_eq!(xfer(&mut d, &[0x10, 0x00, 0xFF])[2], 0x31);
    spi_advance(&mut d, 250);
    assert_eq!(d.take_pin_drives(), vec![("DRDY".to_string(), false)]);
    // WREG MUX = 23h (AIN2 − AIN3 = 2.0 − 0.5 V), ADCON PGA = 2.
    xfer(&mut d, &[0x51, 0x01, 0x23, 0x21]);
    assert_eq!(xfer(&mut d, &[0x11, 0x01, 0xFF, 0xFF])[2..], [0x23, 0x21]);
    // 1.5 V · 2 · 8388607 / 5 V = 5033164.2 → 0x4CCCCC.
    assert_eq!(xfer(&mut d, &[0x01, 0, 0, 0])[1..], [0x4C, 0xCC, 0xCC]);
    // A full read releases DRDY, and the next conversion brings it back.
    assert_eq!(d.take_pin_drives(), vec![("DRDY".to_string(), true)]);
    spi_advance(&mut d, 40);
    assert_eq!(d.take_pin_drives(), vec![("DRDY".to_string(), false)]);
    // Negative input, clipped: AINCOM (0 V) − AIN2 at PGA 64 → 800000h.
    xfer(&mut d, &[0x51, 0x00, 0x82]);
    xfer(&mut d, &[0x52, 0x00, 0x06]);
    assert_eq!(xfer(&mut d, &[0x01, 0, 0, 0])[1..], [0x80, 0x00, 0x00]);
    // STATUS keeps its read-only ID and DRDY bits; ADCON bit 7 reads 0.
    xfer(&mut d, &[0x50, 0x00, 0x0F]);
    xfer(&mut d, &[0x52, 0x00, 0xFF]);
    let r = xfer(&mut d, &[0x10, 0x02, 0xFF, 0xFF, 0xFF]);
    assert_eq!(r[2] & 0xFE, 0x3E);
    assert_eq!(r[4], 0x7F);
    // RREG past 0Ah wraps to 00h.
    let r = xfer(&mut d, &[0x1A, 0x01, 0xFF, 0xFF]);
    assert_eq!(r[3] & 0xF0, 0x30, "wrapped to STATUS");
    // RESET: back to Table 23, DRDY high until settled.
    xfer(&mut d, &[0xFE]);
    assert_eq!(
        xfer(&mut d, &[0x10, 0x04, 0, 0, 0, 0, 0])[2..],
        [0x31, 0x01, 0x60, 0xF0, 0xE0]
    );
}

#[test]
fn sx1262_tx_done_drives_dio1_and_sleep_holds_busy() {
    let mut d = spi_dev("sx1262");
    spi_advance(&mut d, 10);
    let drives = d.take_pin_drives();
    assert!(
        drives.contains(&("BUSY".to_string(), false)),
        "BUSY low once up: {drives:?}"
    );
    // SetDioIrqParams: IrqMask = TxDone, DIO1Mask = TxDone.
    xfer(
        &mut d,
        &[0x08, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
    );
    xfer(&mut d, &[0x8A, 0x01]);
    xfer(&mut d, &[0x83, 0x00, 0x00, 0x00]);
    assert_eq!(d.take_pin_drives(), vec![("DIO1".to_string(), true)]);
    // STBY_RC (0x2) with CommandStatus "TX done" (0x6) → 0x2C.
    assert_eq!(xfer(&mut d, &[0xC0, 0x00])[1], 0x2C);
    assert_eq!(xfer(&mut d, &[0x12, 0, 0, 0])[2..], [0x00, 0x01]);
    xfer(&mut d, &[0x02, 0xFF, 0xFF]);
    assert_eq!(xfer(&mut d, &[0x12, 0, 0, 0])[2..], [0x00, 0x00]);
    assert_eq!(d.take_pin_drives(), vec![("DIO1".to_string(), false)]);
    // RX and FS modes show in ChipMode.
    xfer(&mut d, &[0x82, 0xFF, 0xFF, 0xFF]);
    assert_eq!(xfer(&mut d, &[0xC0, 0x00])[1], 0x50);
    xfer(&mut d, &[0xC1]);
    assert_eq!(xfer(&mut d, &[0xC0, 0x00])[1], 0x40);
    // RSSI: −RssiInst/2 dBm.
    assert_eq!(xfer(&mut d, &[0x15, 0, 0])[2], 190);
    run_spi(&mut d, &[Step::Input("rssi_dbm", -120.0)]);
    assert_eq!(xfer(&mut d, &[0x15, 0, 0])[2], 240);
    // GetPacketStatus (LoRa): RssiPkt, SnrPkt (×4), SignalRssiPkt.
    assert_eq!(xfer(&mut d, &[0x14, 0, 0, 0, 0])[2..], [176, 28, 176]);
    // Public-network sync word written and read back; an undocumented address
    // reads 0 and ignores the write.
    xfer(&mut d, &[0x0D, 0x07, 0x40, 0x34, 0x44]);
    assert_eq!(
        xfer(&mut d, &[0x1D, 0x07, 0x40, 0, 0, 0])[4..],
        [0x34, 0x44]
    );
    xfer(&mut d, &[0x0D, 0x01, 0x00, 0xAA]);
    assert_eq!(xfer(&mut d, &[0x1D, 0x01, 0x00, 0, 0])[4], 0x00);
    // Sleep: BUSY high; the next NSS↓ wakes the part into STBY_RC.
    xfer(&mut d, &[0x84, 0x04]);
    assert_eq!(d.take_pin_drives(), vec![("BUSY".to_string(), true)]);
    // STBY_RC again (SetFs above cleared the command status).
    assert_eq!(xfer(&mut d, &[0xC0, 0x00])[1], 0x20);
    assert_eq!(d.take_pin_drives(), vec![("BUSY".to_string(), false)]);
}

// ─── SK6812RGBW: the single wire, 32 bits per LED ──────────────────────────
//
// No bus and no register: the part's "register" is the colour it latched from
// edge timing on one pad. The check drives a frame at the datasheet's own §10
// timing and reads the decoded LEDs back; the negative controls run the SAME
// check against the WS2812 decoder (24 bits per LED — it must misread an RGBW
// frame) and against a strip listening on another pad (it hears nothing).

use labwired_core::inspect::InspectOpts;
use labwired_core::peripherals::components::declarative_led_strip;
use labwired_core::peripherals::components::GenericLedStrip;
use labwired_core::peripherals::device::GpioObserver;

const LED_PIN: u8 = 18;
/// 80 MHz: 1 cycle = 12.5 ns, so §10's T0H 0.3 µs = 24 cycles, T1H 0.6 µs =
/// 48, T0L 0.9 µs = 72, T1L 0.6 µs = 48, Trst 80 µs = 6400.
const LED_HZ: u64 = 80_000_000;

fn rgbw_strip(device_type: &str, pin: u8, pixels: usize) -> GenericLedStrip {
    let mut s = declarative_led_strip::embedded(device_type).expect("strip descriptor builds");
    s.set_data_pin(pin);
    s.set_num_pixels(pixels);
    s.set_cpu_hz(LED_HZ);
    s
}

/// Clock `leds` (each R, G, B, W — §13 order, MSB first) onto `pin`, then hold
/// the line low for Trst and start one more bit, which is what latches.
/// Starts at `start` sim cycles, so successive frames move forward in time.
fn drive_rgbw(strip: &GenericLedStrip, start: u64, leds: &[[u8; 4]]) {
    let mut cycle = start;
    let mut level = false;
    let mut edge = |to: bool, at: u64| {
        GpioObserver::on_pin_change(strip, LED_PIN, level, to, at);
        level = to;
    };
    for led in leds {
        let word = u32::from_be_bytes(*led);
        for i in (0..32).rev() {
            let (high, low) = if (word >> i) & 1 != 0 {
                (48, 48)
            } else {
                (24, 72)
            };
            edge(true, cycle);
            cycle += high;
            edge(false, cycle);
            cycle += low;
        }
    }
    edge(true, cycle + 6_400);
    edge(false, cycle + 6_424);
}

/// SK6812RGBW (OPSCO SPC/SK6812RGBW Rev 01 §10/§13): two LEDs, one with all
/// four channels distinct and one lit by its white die alone, decode to their
/// wire bytes, and the artifact carries four bytes per LED.
fn check_sk6812(strip: &GenericLedStrip) -> Result<(), String> {
    drive_rgbw(
        strip,
        1_000,
        &[[0x12, 0x34, 0x56, 0x78], [0x00, 0x00, 0x00, 0xFF]],
    );
    let px = strip.pixels();
    ensure_eq!(px.len(), 2usize, "SK6812 LEDs decoded");
    ensure_eq!(
        (px[0].wire, px[0].white),
        ([0x12u8, 0x34, 0x56], Some(0x78u8)),
        "SK6812 LED 0 RGB + W"
    );
    ensure_eq!(
        (px[1].wire, px[1].white),
        ([0u8, 0, 0], Some(0xFFu8)),
        "SK6812 LED 1 white only"
    );
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let art = SpiDevice::artifacts(strip, "strip", &opts);
    let a = art.first().ok_or("SK6812 published no artifact")?;
    ensure_eq!(
        a.meta["format"].as_str(),
        Some("sk6812_rgbw"),
        "SK6812 artifact format"
    );
    ensure_eq!(
        a.meta["lit_pixels"].as_u64(),
        Some(2u64),
        "SK6812 lit pixels (W counts)"
    );
    ensure_eq!(
        a.bytes.clone(),
        Some(vec![0x12u8, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0xFF]),
        "SK6812 artifact bytes"
    );
    Ok(())
}

#[test]
fn sk6812_passes_its_smoke_check() {
    check_sk6812(&rgbw_strip("sk6812", LED_PIN, 4)).unwrap();
    println!("smoke  sk6812    (GPIO NRZ, 32 bit): PASS");
}

#[test]
fn negative_control_a_24_bit_or_deaf_strip_fails_the_sk6812_check() {
    for (what, strip) in [
        ("WS2812 24-bit decoder", rgbw_strip("neopixel", LED_PIN, 4)),
        (
            "SK6812 on another pad",
            rgbw_strip("sk6812", LED_PIN + 1, 4),
        ),
    ] {
        match check_sk6812(&strip) {
            Err(e) => println!("negctl sk6812 vs {what}: FAILS as required ({e})"),
            Ok(()) => panic!("{what} PASSED the SK6812 check — the check proves nothing"),
        }
    }
}

#[test]
fn sk6812_tracks_successive_frames() {
    let strip = rgbw_strip("sk6812", LED_PIN, 3);
    drive_rgbw(
        &strip,
        1_000,
        &[[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
    );
    let px = strip.pixels();
    assert_eq!(
        px.iter().map(|p| (p.wire, p.white)).collect::<Vec<_>>(),
        vec![
            ([1, 2, 3], Some(4)),
            ([5, 6, 7], Some(8)),
            ([9, 10, 11], Some(12)),
        ]
    );
    // A second frame replaces the first.
    drive_rgbw(&strip, 1_000_000, &[[0xFF, 0, 0, 0]]);
    assert_eq!(strip.pixels()[0].wire, [0xFF, 0, 0]);
    assert_eq!(strip.pixels()[0].white, Some(0));
    // The WS2812 strip's pixels carry no white byte: three-colour parts are
    // unchanged by the 32-bit decoder.
    let ws = rgbw_strip("neopixel", LED_PIN, 1);
    drive_rgbw(&ws, 1_000, &[[1, 2, 3, 4]]);
    assert!(ws.pixels().iter().all(|p| p.white.is_none()));
}

// ─── TM1638: three bit-banged GPIO lines ───────────────────────────────────
//
// A write-only display driver: its "registers" are the 16 display-RAM bytes,
// and the only place they surface is the artifact. The check therefore runs
// the part on a real STM32F103 bus, bit-banging STB/CLK/DIO through BSRR the
// way a TM1638 library does, and reads the inspected artifact. The negative
// controls run the SAME check against a TM1637 on the same pins (a different
// protocol must not decode as this one) and against a TM1638 wired to pads the
// script never moves (it must stay blank).

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{Artifact, DeviceInspect};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};

const GPIOA_BSRR: u64 = 0x4001_0800 + 0x10;
const RCC_APB2ENR: u64 = 0x4002_1018;
const STB: u32 = 7; // PA7
const TCLK: u32 = 8; // PA8
const TDIO: u32 = 9; // PA9

fn gpio_rig(device_type: &str, config: &[(&str, &str)]) -> SystemBus {
    let chip_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "tm1638-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "panel".to_string(),
            r#type: device_type.to_string(),
            connection: "gpio".to_string(),
            channel: None,
            route: Default::default(),
            config: config
                .iter()
                .map(|(k, v)| (k.to_string(), serde_yaml::Value::from(*v)))
                .collect(),
        }],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    bus.write_u32(RCC_APB2ENR, 0xFFFF).expect("ungate GPIOA");
    bus
}

/// One BSRR store setting all three lines at once.
fn lines(bus: &mut SystemBus, stb: bool, clk: bool, dio: bool) {
    let (mut set, mut clear) = (0u32, 0u32);
    for (bit, level) in [(STB, stb), (TCLK, clk), (TDIO, dio)] {
        if level {
            set |= 1 << bit;
        } else {
            clear |= 1 << bit;
        }
    }
    bus.write_u32(GPIOA_BSRR, set | (clear << 16))
        .expect("BSRR store");
}

/// STB↓, the bytes LSB first on CLK↑ (§IX), STB↑ — one TM1638 instruction.
fn tm1638_send(bus: &mut SystemBus, bytes: &[u8]) {
    lines(bus, true, true, true);
    lines(bus, false, true, true);
    for &b in bytes {
        for i in 0..8 {
            let bit = (b >> i) & 1 != 0;
            lines(bus, false, false, bit);
            lines(bus, false, true, bit);
        }
    }
    lines(bus, true, true, true);
}

fn panel_artifact(bus: SystemBus) -> Result<Artifact, String> {
    let mut bus = bus;
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.refresh_peripheral_index();
    let machine = Machine::new(cpu, bus);
    let device: DeviceInspect = machine
        .inspect(None, &InspectOpts::default())
        .devices
        .into_iter()
        .find(|d| d.id == "panel")
        .ok_or("no device 'panel'")?;
    device
        .artifacts
        .into_iter()
        .next()
        .ok_or_else(|| "the panel published no artifact".into())
}

const TM_PINS: &[(&str, &str)] = &[("stb_pin", "PA7"), ("clk_pin", "PA8"), ("dio_pin", "PA9")];

/// TM1638 (Titan Micro V1.3 §V/§VI): data command 0x40 (write, auto-increment),
/// address command 0xC0 plus 16 bytes — digits "01234567" on the even
/// addresses, LEDs 1, 3 and 8 on bit 0 of the odd ones — then display control
/// 0x8F (on, 14/16). The artifact must show the digits, the LEDs, the switch
/// and the pulse width.
fn check_tm1638(mut bus: SystemBus) -> Result<(), String> {
    tm1638_send(&mut bus, &[0x40]);
    tm1638_send(
        &mut bus,
        &[
            0xC0, 0x3F, 0x01, 0x06, 0x00, 0x5B, 0x01, 0x4F, 0x00, 0x66, 0x00, 0x6D, 0x00, 0x7D,
            0x00, 0x07, 0x01,
        ],
    );
    tm1638_send(&mut bus, &[0x8F]);
    let a = panel_artifact(bus)?;
    ensure_eq!(a.kind.as_str(), "text_display", "TM1638 artifact kind");
    ensure_eq!(
        a.meta["format"].as_str(),
        Some("tm1638_grid"),
        "TM1638 artifact format"
    );
    ensure_eq!(a.meta["text"].as_str(), Some("01234567"), "TM1638 digits");
    ensure_eq!(
        a.meta["leds"].as_u64(),
        Some(0x85u64),
        "TM1638 discrete LEDs (bit 0 of odd addresses)"
    );
    ensure_eq!(
        a.meta["display_on"].as_bool(),
        Some(true),
        "TM1638 display switch"
    );
    ensure_eq!(
        a.meta["pulse_width"].as_u64(),
        Some(7u64),
        "TM1638 pulse width"
    );
    Ok(())
}

#[test]
fn tm1638_passes_its_smoke_check() {
    check_tm1638(gpio_rig("tm1638", TM_PINS)).unwrap();
    println!("smoke  tm1638    (GPIO STB/CLK/DIO on F103): PASS");
}

#[test]
fn negative_control_a_tm1637_or_deaf_panel_fails_the_tm1638_check() {
    for (what, bus) in [
        (
            "TM1637 on CLK/DIO",
            gpio_rig("tm1637-7seg", &[("clk_pin", "PA8"), ("dio_pin", "PA9")]),
        ),
        (
            "TM1638 on unused pads",
            gpio_rig(
                "tm1638",
                &[("stb_pin", "PA0"), ("clk_pin", "PA1"), ("dio_pin", "PA2")],
            ),
        ),
    ] {
        match check_tm1638(bus) {
            Err(e) => println!("negctl tm1638 vs {what}: FAILS as required ({e})"),
            Ok(()) => panic!("{what} PASSED the TM1638 check — the check proves nothing"),
        }
    }
}

#[test]
fn tm1638_fixed_address_out_of_range_and_abandoned_bytes() {
    let mut bus = gpio_rig("tm1638", TM_PINS);
    // Fixed address (0x44): two writes to 06H, the second wins; 00H untouched.
    tm1638_send(&mut bus, &[0x44]);
    tm1638_send(&mut bus, &[0xC6, 0x06]);
    tm1638_send(&mut bus, &[0xC6, 0x5B]);
    // Auto-increment from 0EH: 0EH and 0FH land, the third byte (10H) is
    // ignored rather than wrapping onto 00H.
    tm1638_send(&mut bus, &[0x40]);
    tm1638_send(&mut bus, &[0xCE, 0x4F, 0x01, 0x3F]);
    // STB↑ after four bits of an address command abandons it: nothing changes.
    lines(&mut bus, true, true, true);
    lines(&mut bus, false, true, true);
    for i in 0..4 {
        let bit = (0xC0u8 >> i) & 1 != 0;
        lines(&mut bus, false, false, bit);
        lines(&mut bus, false, true, bit);
    }
    lines(&mut bus, true, true, true);
    tm1638_send(&mut bus, &[0x88]); // on, 1/16
    let a = panel_artifact(bus).unwrap();
    // 00H blank, 06H = '2', 0EH = '3'; the font renders a blank digit as ' '.
    assert_eq!(a.meta["text"].as_str(), Some("   2   3"));
    assert_eq!(a.meta["leds"].as_u64(), Some(0x80));
    assert_eq!(a.meta["pulse_width"].as_u64(), Some(0));
}
