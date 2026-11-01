// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The everyday sensor set: one smoke check per part, and the negative
//! control that keeps each check honest.**
//!
//! Every part below is a `configs/devices/*.yaml` descriptor with no Rust
//! model. Each one gets a `check_*` function that talks to it the way its
//! datasheet (and its common drivers) do and asserts REAL register values: the
//! device ID where the part has one, a measurement word at the default
//! stimulus, and the protocol's own integrity bytes (Sensirion CRC-8, the
//! Plantower check code).
//!
//! The negative control is the reason the checks are functions rather than
//! inline asserts. [`negative_control_a_silent_part_fails_every_check`] runs
//! each check against a stand-in that answers every read with `0x00`, and
//! another that answers `0xFF` (an open bus — nothing ACKed). Both must FAIL
//! every check. A smoke test that a zero-answering part could pass proves
//! nothing, and this file refuses to contain one.
//!
//! The `*_tracks_stimulus` tests then move the inputs and the configuration and
//! check the words move by the datasheet's arithmetic, so a part that answered
//! a constant frame would fail those even if the constant were right.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::Bus;
use std::path::PathBuf;

mod common;
use common::transcript::{
    read_reg, read_stream, run_i2c, run_spi, script, spi_xfer, write_reg, Step,
};

// ─── plumbing ──────────────────────────────────────────────────────────────

fn dev(device_type: &str, addr: u8) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    GenericI2cDevice::from_yaml(yaml, addr).unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

/// Sensirion / Aosong CRC-8: poly 0x31, init 0xFF, no reflection, no final XOR.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0xFFu8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn rd(d: &mut dyn I2cDevice, ptr: u8, n: usize) -> Vec<u8> {
    run_i2c(d, &read_reg(ptr, n)).bytes
}

fn wr(d: &mut dyn I2cDevice, ptr: u8, bytes: &[u8]) {
    run_i2c(d, &write_reg(ptr, bytes));
}

/// Write raw bytes in one transaction (a command plus its parameters).
fn send(d: &mut dyn I2cDevice, bytes: &[u8]) {
    let mut s = vec![Step::Start];
    s.extend(bytes.iter().map(|&b| Step::Write(b)));
    s.push(Step::Stop);
    run_i2c(d, &s);
}

fn advance(d: &mut dyn I2cDevice, us: u64) {
    run_i2c(d, &[Step::AdvanceUs(us)]);
}

fn bare_read(d: &mut dyn I2cDevice, n: usize) -> Vec<u8> {
    run_i2c(d, &read_stream(n)).bytes
}

fn be16(b: &[u8]) -> u16 {
    u16::from(b[0]) << 8 | u16::from(b[1])
}

fn le16(b: &[u8]) -> u16 {
    u16::from(b[1]) << 8 | u16::from(b[0])
}

/// Split a Sensirion response into its 16-bit words, verifying each CRC.
fn sensirion_words(bytes: &[u8]) -> Result<Vec<u16>, String> {
    if !bytes.len().is_multiple_of(3) {
        return Err(format!(
            "{} bytes is not a whole number of word+CRC triples",
            bytes.len()
        ));
    }
    bytes
        .chunks(3)
        .map(|c| {
            if crc8(&c[..2]) == c[2] {
                Ok(be16(c))
            } else {
                Err(format!(
                    "CRC mismatch on word {:02X}{:02X}: got {:02X}, want {:02X}",
                    c[0],
                    c[1],
                    c[2],
                    crc8(&c[..2])
                ))
            }
        })
        .collect()
}

macro_rules! ensure_eq {
    ($got:expr, $want:expr, $($what:tt)+) => {{
        let (g, w) = ($got, $want);
        if g != w {
            return Err(format!("{}: got {:02X?}, want {:02X?}", format!($($what)+), g, w));
        }
    }};
}

// ─── the checks (default stimulus) ─────────────────────────────────────────

/// SHT40: 0xFD high-precision measurement, then the serial number.
/// 25 °C → (25+45)·65535/175 = 26214 = 0x6666; 50 %RH → (50+6)·65535/125 =
/// 29359.68 → 0x72B0.
fn check_sht40(d: &mut dyn I2cDevice) -> Result<(), String> {
    send(d, &[0xFD]);
    advance(d, 8_300);
    let w = sensirion_words(&bare_read(d, 6))?;
    ensure_eq!(w[0], 0x6666u16, "SHT40 temperature word at 25 °C");
    ensure_eq!(w[1], 0x72B0u16, "SHT40 humidity word at 50 %RH");
    send(d, &[0x89]);
    let s = sensirion_words(&bare_read(d, 6))?;
    ensure_eq!(s, vec![0x0F4Eu16, 0x2A71], "SHT40 serial number");
    Ok(())
}

/// SGP40: measure_raw with the datasheet's default compensation words
/// (50 %RH = 0x8000 / CRC 0xA2, 25 °C = 0x6666 / CRC 0x93), then self-test.
fn check_sgp40(d: &mut dyn I2cDevice) -> Result<(), String> {
    send(d, &[0x26, 0x0F, 0x80, 0x00, 0xA2, 0x66, 0x66, 0x93]);
    advance(d, 30_000);
    let w = sensirion_words(&bare_read(d, 3))?;
    ensure_eq!(
        w,
        vec![28_000u16],
        "SGP40 SRAW_VOC at the default 28000 ticks"
    );
    send(d, &[0x28, 0x0E]);
    advance(d, 320_000);
    let w = sensirion_words(&bare_read(d, 3))?;
    ensure_eq!(w, vec![0xD400u16], "SGP40 self-test result (all passed)");
    Ok(())
}

/// DHT20: the manual's sequence — 0x71 status, 0xAC 0x33 0x00 trigger, wait
/// 80 ms, read seven bytes. 50 %RH → 2^19 = 0x80000; 25 °C → 75/200·2^20 =
/// 0x60000; packed 40-bit frame 0x80000_60000.
fn check_dht20(d: &mut dyn I2cDevice) -> Result<(), String> {
    send(d, &[0x71]);
    ensure_eq!(
        bare_read(d, 1),
        vec![0x18u8],
        "DHT20 status after 0x71 (calibrated, idle)"
    );
    send(d, &[0xAC, 0x33, 0x00]);
    advance(d, 80_000);
    let f = bare_read(d, 7);
    ensure_eq!(f[0], 0x18u8, "DHT20 status byte of the measurement frame");
    ensure_eq!(
        f[6],
        crc8(&f[..6]),
        "DHT20 CRC over the six preceding bytes"
    );
    let raw = f[1..6].iter().fold(0u64, |a, &b| a << 8 | u64::from(b));
    ensure_eq!(raw >> 20, 0x80000u64, "DHT20 humidity field at 50 %RH");
    ensure_eq!(
        raw & 0xFFFFF,
        0x60000u64,
        "DHT20 temperature field at 25 °C"
    );
    Ok(())
}

/// TSL2591: ID through the command byte (0xA0 | 0x12), then power on, wait one
/// integration, and read both channels in one burst from 0xB4.
fn check_tsl2591(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0xB2, 1), vec![0x50u8], "TSL2591 ID register");
    wr(d, 0xA0, &[0x03]); // ENABLE = AEN | PON
    advance(d, 100_000);
    ensure_eq!(
        rd(d, 0xB3, 1)[0] & 0x01,
        0x01u8,
        "TSL2591 STATUS.AVALID after one integration"
    );
    let c = rd(d, 0xB4, 4);
    ensure_eq!(le16(&c[0..2]), 1200u16, "TSL2591 CH0 at 1× / 100 ms");
    ensure_eq!(le16(&c[2..4]), 300u16, "TSL2591 CH1 at 1× / 100 ms");
    Ok(())
}

/// INA226: the ID registers, the power-on configuration, and the measurement
/// words at 5 V / 0.25 A through 0.1 Ω (shunt 10000 counts, bus 4000 counts).
fn check_ina226(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(be16(&rd(d, 0xFE, 2)), 0x5449u16, "INA226 manufacturer ID");
    ensure_eq!(be16(&rd(d, 0xFF, 2)), 0x2260u16, "INA226 die ID");
    ensure_eq!(
        be16(&rd(d, 0x00, 2)),
        0x4127u16,
        "INA226 configuration POR value"
    );
    ensure_eq!(
        be16(&rd(d, 0x01, 2)),
        10_000u16,
        "INA226 shunt voltage (2.5 µV/LSB)"
    );
    ensure_eq!(
        be16(&rd(d, 0x02, 2)),
        4_000u16,
        "INA226 bus voltage (1.25 mV/LSB)"
    );
    Ok(())
}

/// PMSA003I: a bare 32-byte read is the whole Plantower frame.
fn check_pmsa003i(d: &mut dyn I2cDevice) -> Result<(), String> {
    let f = bare_read(d, 32);
    ensure_eq!(&f[0..2], &[0x42u8, 0x4D][..], "PMSA003I start characters");
    ensure_eq!(be16(&f[2..4]), 28u16, "PMSA003I frame length");
    ensure_eq!(
        be16(&f[12..14]),
        8u16,
        "PMSA003I PM2.5 (atmospheric) at 8 µg/m³"
    );
    ensure_eq!(be16(&f[16..18]), 900u16, "PMSA003I >0.3 µm count");
    let sum: u16 = f[..30].iter().map(|&b| u16::from(b)).sum();
    ensure_eq!(
        be16(&f[30..32]),
        sum,
        "PMSA003I check code = byte sum of 0x00..0x1D"
    );
    Ok(())
}

/// ICM-42688-P: WHO_AM_I, the "sensor off" value, then accel Z at 1 g on the
/// reset ±16 g range (2048 LSB/g).
fn check_icm42688(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x75, 1), vec![0x47u8], "ICM-42688 WHO_AM_I");
    ensure_eq!(
        be16(&rd(d, 0x23, 2)),
        0x8000u16,
        "ICM-42688 accel Z while the accel is OFF"
    );
    wr(d, 0x4E, &[0x0F]); // PWR_MGMT0: gyro LN + accel LN
    ensure_eq!(
        be16(&rd(d, 0x23, 2)),
        2048u16,
        "ICM-42688 accel Z at 1 g, ±16 g"
    );
    Ok(())
}

/// ICM-20948: WHO_AM_I, asleep-reads-zero, wake, accel Z at 1 g on the reset
/// ±2 g range (16384 LSB/g), and a bank-2 register through REG_BANK_SEL.
fn check_icm20948(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x00, 1), vec![0xEAu8], "ICM-20948 WHO_AM_I");
    ensure_eq!(
        rd(d, 0x06, 1),
        vec![0x41u8],
        "ICM-20948 PWR_MGMT_1 reset (SLEEP | CLKSEL=1)"
    );
    wr(d, 0x06, &[0x01]); // clear SLEEP
    ensure_eq!(
        be16(&rd(d, 0x31, 2)),
        16384u16,
        "ICM-20948 accel Z at 1 g, ±2 g"
    );
    wr(d, 0x7F, &[0x20]); // bank 2
    ensure_eq!(
        rd(d, 0x14, 1),
        vec![0x01u8],
        "ICM-20948 bank-2 ACCEL_CONFIG reset"
    );
    wr(d, 0x7F, &[0x00]);
    Ok(())
}

/// BME680: CHIP_ID, Bosch's init (soft reset, calibration burst reads), one
/// forced measurement, and the Bosch integer compensation of the 17-byte
/// field-0 read — which must give back the default stimulus EXACTLY.
fn check_bme680(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0xD0, 1), vec![0x61u8], "BME680 CHIP_ID");
    wr(d, 0xE0, &[0xB6]);
    ensure_eq!(
        rd(d, 0xF0, 1),
        vec![0x00u8],
        "BME680 VARIANT_ID (BME680, not BME688)"
    );
    let cal = bosch::Calib::read(d);
    if cal.par_t1 == 0 || cal.par_p1 == 0 {
        return Err(format!("calibration block reads empty: {cal:?}"));
    }
    let r = bme680_forced(d, &cal)?;
    ensure_eq!(r.t, 2500i16, "BME680 temperature (0.01 °C) at 25 °C");
    ensure_eq!(r.p, 101_325u32, "BME680 pressure (Pa) at 1013.25 hPa");
    // One humidity ADC count is ≈4 milli-%RH through Bosch's formula, so the
    // nearest reachable answer to 50.000 % is within 2 milli-%.
    if r.h.abs_diff(50_000) > 2 {
        return Err(format!("BME680 humidity at 50 %RH: got {} milli-%", r.h));
    }
    Ok(())
}

/// One forced T/P/H cycle the way `bme68x_set_op_mode` + `bme68x_get_data`
/// run it: osrs 1×, forced, wait the measurement, read 0x1D..0x2D, compensate.
struct Bme680Reading {
    t: i16,
    p: u32,
    h: u32,
    adc_p: u32,
    adc_h: u16,
    t_fine: i32,
}

fn bme680_forced(d: &mut dyn I2cDevice, cal: &bosch::Calib) -> Result<Bme680Reading, String> {
    wr(d, 0x72, &[0x01]); // osrs_h = 1×
    wr(d, 0x74, &[0x24]); // osrs_t = 1×, osrs_p = 1×, sleep
    wr(d, 0x74, &[0x25]); // … forced
    advance(d, 7_256);
    let f = rd(d, 0x1D, 17);
    if f[0] & 0x80 == 0 {
        return Err(format!(
            "new_data_0 clear after the measurement: status {:#04X}",
            f[0]
        ));
    }
    ensure_eq!(
        rd(d, 0x74, 1)[0] & 0x03,
        0u8,
        "BME680 back in sleep mode after forced"
    );
    let adc_p = u32::from(f[2]) << 12 | u32::from(f[3]) << 4 | u32::from(f[4]) >> 4;
    let adc_t = u32::from(f[5]) << 12 | u32::from(f[6]) << 4 | u32::from(f[7]) >> 4;
    let adc_h = u16::from(f[8]) << 8 | u16::from(f[9]);
    let (t, t_fine) = bosch::calc_temperature(adc_t, cal);
    Ok(Bme680Reading {
        t,
        p: bosch::calc_pressure(adc_p, t_fine, cal),
        h: bosch::calc_humidity(adc_h, t_fine, cal),
        adc_p,
        adc_h,
        t_fine,
    })
}

/// Bosch BME68x_SensorAPI (`bme68x.c`, integer build) — `get_calib_data`,
/// `calc_temperature`, `calc_pressure`, `calc_humidity`, ported line for line
/// with C's int32 wrapping semantics. This is the oracle the descriptor's
/// `invert` programs must agree with; it reads the calibration off the WIRE,
/// so a wrong byte in the descriptor's coefficient block fails here too.
mod bosch {
    use super::{rd, I2cDevice};

    #[derive(Debug, Default)]
    pub struct Calib {
        pub par_t1: u16,
        pub par_t2: i16,
        pub par_t3: i8,
        pub par_p1: u16,
        pub par_p2: i16,
        pub par_p3: i8,
        pub par_p4: i16,
        pub par_p5: i16,
        pub par_p6: i8,
        pub par_p7: i8,
        pub par_p8: i16,
        pub par_p9: i16,
        pub par_p10: u8,
        pub par_h1: u16,
        pub par_h2: u16,
        pub par_h3: i8,
        pub par_h4: i8,
        pub par_h5: i8,
        pub par_h6: u8,
        pub par_h7: i8,
    }

    impl Calib {
        /// The three burst reads `get_calib_data` issues, and its decode.
        pub fn read(d: &mut dyn I2cDevice) -> Self {
            let mut c = rd(d, 0x8A, 23);
            c.extend(rd(d, 0xE1, 14));
            c.extend(rd(d, 0x00, 5));
            let w = |msb: usize, lsb: usize| u16::from(c[msb]) << 8 | u16::from(c[lsb]);
            Calib {
                par_t1: w(32, 31),
                par_t2: w(1, 0) as i16,
                par_t3: c[2] as i8,
                par_p1: w(5, 4),
                par_p2: w(7, 6) as i16,
                par_p3: c[8] as i8,
                par_p4: w(11, 10) as i16,
                par_p5: w(13, 12) as i16,
                par_p6: c[15] as i8,
                par_p7: c[14] as i8,
                par_p8: w(19, 18) as i16,
                par_p9: w(21, 20) as i16,
                par_p10: c[22],
                par_h1: u16::from(c[25]) << 4 | u16::from(c[24] & 0x0F),
                par_h2: u16::from(c[23]) << 4 | u16::from(c[24] >> 4),
                par_h3: c[26] as i8,
                par_h4: c[27] as i8,
                par_h5: c[28] as i8,
                par_h6: c[29],
                par_h7: c[30] as i8,
            }
        }
    }

    pub fn calc_temperature(temp_adc: u32, c: &Calib) -> (i16, i32) {
        let var1: i64 = i64::from((temp_adc as i32) >> 3) - (i64::from(c.par_t1) << 1);
        let var2: i64 = (var1 * i64::from(c.par_t2)) >> 11;
        let mut var3: i64 = ((var1 >> 1) * (var1 >> 1)) >> 12;
        var3 = (var3 * (i64::from(c.par_t3) << 4)) >> 14;
        let t_fine = (var2 + var3) as i32;
        (((t_fine * 5 + 128) >> 8) as i16, t_fine)
    }

    pub fn calc_pressure(pres_adc: u32, t_fine: i32, c: &Calib) -> u32 {
        let (p1, p2, p3, p4) = (
            i32::from(c.par_p1),
            i32::from(c.par_p2),
            i32::from(c.par_p3),
            i32::from(c.par_p4),
        );
        let (p5, p6, p7, p8) = (
            i32::from(c.par_p5),
            i32::from(c.par_p6),
            i32::from(c.par_p7),
            i32::from(c.par_p8),
        );
        let (p9, p10) = (i32::from(c.par_p9), i32::from(c.par_p10));
        let mut var1: i32 = (t_fine >> 1) - 64000;
        let mut var2: i32 = ((((var1 >> 2).wrapping_mul(var1 >> 2)) >> 11).wrapping_mul(p6)) >> 2;
        var2 = var2.wrapping_add((var1.wrapping_mul(p5)) << 1);
        var2 = (var2 >> 2).wrapping_add(p4 << 16);
        var1 = (((((var1 >> 2).wrapping_mul(var1 >> 2)) >> 13).wrapping_mul(p3 << 5)) >> 3)
            .wrapping_add(p2.wrapping_mul(var1) >> 1);
        var1 >>= 18;
        var1 = ((32768 + var1).wrapping_mul(p1)) >> 15;
        let mut pc: i32 = 1_048_576i32.wrapping_sub(pres_adc as i32);
        pc = (pc.wrapping_sub(var2 >> 12) as u32).wrapping_mul(3125) as i32;
        pc = if pc >= 0x4000_0000 {
            (pc / var1) << 1
        } else {
            (pc << 1) / var1
        };
        var1 = p9.wrapping_mul(((pc >> 3).wrapping_mul(pc >> 3)) >> 13) >> 12;
        var2 = ((pc >> 2).wrapping_mul(p8)) >> 13;
        let var3 = (pc >> 8)
            .wrapping_mul(pc >> 8)
            .wrapping_mul(pc >> 8)
            .wrapping_mul(p10)
            >> 17;
        pc = pc.wrapping_add((var1 + var2 + var3 + (p7 << 7)) >> 4);
        pc as u32
    }

    pub fn calc_humidity(hum_adc: u16, t_fine: i32, c: &Calib) -> u32 {
        let (h1, h2) = (i32::from(c.par_h1), i32::from(c.par_h2));
        let (h3, h4, h5) = (
            i32::from(c.par_h3),
            i32::from(c.par_h4),
            i32::from(c.par_h5),
        );
        let (h6, h7) = (i32::from(c.par_h6), i32::from(c.par_h7));
        let ts: i32 = (t_fine * 5 + 128) >> 8;
        let var1 = (i32::from(hum_adc) - h1 * 16) - (((ts * h3) / 100) >> 1);
        let var2 =
            (h2 * (((ts * h4) / 100) + (((ts * ((ts * h5) / 100)) >> 6) / 100) + (1 << 14))) >> 10;
        let var3 = var1.wrapping_mul(var2);
        let mut var4 = h6 << 7;
        var4 = (var4 + ((ts * h7) / 100)) >> 4;
        let var5 = ((var3 >> 14).wrapping_mul(var3 >> 14)) >> 10;
        let var6 = var4.wrapping_mul(var5) >> 1;
        let calc = ((var3.wrapping_add(var6) >> 10).wrapping_mul(1000)) >> 12;
        calc.clamp(0, 100_000) as u32
    }
}

// ─── batch 4: expanders and the ADS1015 (I²C) ─────────────────────────────

/// MCP23017 (DS20001952D Table 3-5): the whole BANK = 0 map in one sequential
/// burst — IODIRA/B power up 0xFF, everything else 0x00 — then the pointer's
/// 0x15 → 0x00 rollover (§3.2.1), IOCON's two addresses (bit 0 unimplemented)
/// and the GPIO → OLAT path with the pin turned to output.
fn check_mcp23017(d: &mut dyn I2cDevice) -> Result<(), String> {
    let mut por = vec![0xFFu8, 0xFF];
    por.extend([0x00u8; 20]);
    ensure_eq!(rd(d, 0x00, 22), por, "MCP23017 power-on map 0x00..0x15");
    ensure_eq!(
        rd(d, 0x15, 2),
        vec![0x00u8, 0xFF],
        "MCP23017 OLATB then rollover to IODIRA"
    );
    // MIRROR (bit 6) plus the unimplemented bit 0. Not SEQOP: setting that
    // would put silicon in Byte mode and stop the burst below incrementing.
    wr(d, 0x0A, &[0x41]);
    ensure_eq!(
        rd(d, 0x0B, 1),
        vec![0x40u8],
        "MCP23017 IOCON written at 0x0A, read at 0x0B (bit 0 reads 0)"
    );
    wr(d, 0x00, &[0x00]); // IODIRA: all outputs
    wr(d, 0x12, &[0xA5]); // GPIOA write lands in OLATA
    ensure_eq!(
        rd(d, 0x12, 3),
        vec![0xA5u8, 0x00, 0xA5],
        "MCP23017 GPIOA / GPIOB / OLATA after a GPIOA write"
    );
    Ok(())
}

/// PCF8575 (SCPS121I §8.1): power-on port reads 0xFF 0xFF; a written pair is
/// read back port 0 first, and the pairing repeats for as long as the master
/// clocks.
fn check_pcf8575(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(bare_read(d, 2), vec![0xFFu8, 0xFF], "PCF8575 power-on port");
    send(d, &[0x34, 0x12]);
    ensure_eq!(
        bare_read(d, 4),
        vec![0x34u8, 0x12, 0x34, 0x12],
        "PCF8575 port 0, port 1, port 0, port 1 after writing 0x1234"
    );
    Ok(())
}

/// ADS1015 (SBAS473F §8): CONFIG POR 8583h, Lo_thresh 8000h, Hi_thresh 7FFFh,
/// and the reserved low nibbles of the thresholds (R-0h / R-Fh) surviving a
/// write. CONVERSION is exercised in `ads1015_tracks_stimulus`.
fn check_ads1015(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x01, 2), vec![0x85u8, 0x83], "ADS1015 CONFIG POR");
    ensure_eq!(rd(d, 0x02, 2), vec![0x80u8, 0x00], "ADS1015 Lo_thresh POR");
    ensure_eq!(rd(d, 0x03, 2), vec![0x7Fu8, 0xFF], "ADS1015 Hi_thresh POR");
    wr(d, 0x02, &[0x12, 0x3F]);
    wr(d, 0x03, &[0x12, 0x30]);
    ensure_eq!(
        rd(d, 0x02, 2),
        vec![0x12u8, 0x30],
        "ADS1015 Lo_thresh keeps its R-0h nibble"
    );
    ensure_eq!(
        rd(d, 0x03, 2),
        vec![0x12u8, 0x3F],
        "ADS1015 Hi_thresh keeps its R-Fh nibble"
    );
    // OS (D15) cannot be cleared by a write: it reads "not converting".
    wr(d, 0x01, &[0x44, 0x83]);
    ensure_eq!(
        rd(d, 0x01, 2),
        vec![0xC4u8, 0x83],
        "ADS1015 CONFIG write (MUX 100, PGA 010) with OS still set"
    );
    Ok(())
}

type Check = fn(&mut dyn I2cDevice) -> Result<(), String>;

const PARTS: &[(&str, u8, Check)] = &[
    ("sht40", 0x44, check_sht40),
    ("bme680", 0x76, check_bme680),
    ("icm42688", 0x68, check_icm42688),
    ("icm20948", 0x68, check_icm20948),
    ("sgp40", 0x59, check_sgp40),
    ("tsl2591", 0x29, check_tsl2591),
    ("dht20", 0x38, check_dht20),
    ("pmsa003i", 0x12, check_pmsa003i),
    ("ina226", 0x40, check_ina226),
    ("mcp23017", 0x20, check_mcp23017),
    ("pcf8575", 0x20, check_pcf8575),
    ("ads1015", 0x48, check_ads1015),
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
    assert!(
        failures.is_empty(),
        "smoke failures:\n{}",
        failures.join("\n")
    );
}

/// A stand-in part that ACKs everything and answers every read with one byte.
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

#[test]
fn negative_control_a_silent_part_fails_every_check() {
    for &(ty, _, check) in PARTS {
        for byte in [0x00u8, 0xFF] {
            let got = check(&mut Silent { byte });
            match got {
                Err(e) => {
                    println!("negctl {ty:<9} answering 0x{byte:02X}: FAILS as required ({e})")
                }
                Ok(()) => panic!(
                    "{ty}: a part answering only 0x{byte:02X} PASSED the smoke check — the \
                     check proves nothing"
                ),
            }
        }
    }
}

// ─── per-part behaviour beyond the default stimulus ────────────────────────

#[test]
fn sht40_waits_for_its_conversion_and_tracks_stimulus() {
    let mut d = dev("sht40", 0x44);
    send(&mut d, &[0xFD]);
    advance(&mut d, 8_299);
    assert_eq!(
        bare_read(&mut d, 6),
        vec![0xFF; 6],
        "read 1 µs before the 8.3 ms max"
    );
    let mut d = dev("sht40", 0x44);
    let got = run_i2c(
        &mut d,
        &script([
            vec![
                Step::Input("temperature", -10.0),
                Step::Input("humidity", 0.0),
            ],
            vec![
                Step::Start,
                Step::Write(0xE0),
                Step::Stop,
                Step::AdvanceUs(1_600),
            ],
            read_stream(6),
        ]),
    );
    let w = sensirion_words(&got.bytes).unwrap();
    // −10 °C → 35·65535/175 = 13107; 0 %RH → 6·65535/125 = 3145.68 → 3146.
    assert_eq!(w, vec![13_107, 3_146]);
    // An SHT3x opcode is not an SHT40 command: nothing is queued.
    send(&mut d, &[0x24, 0x00]);
    assert_eq!(bare_read(&mut d, 3), vec![0xFF; 3]);
}

#[test]
fn sgp40_tracks_its_raw_signal() {
    let mut d = dev("sgp40", 0x59);
    let got = run_i2c(
        &mut d,
        &script([
            vec![Step::Input("voc_sraw", 31_234.0)],
            vec![
                Step::Start,
                Step::Write(0x26),
                Step::Write(0x0F),
                Step::Write(0x80),
                Step::Write(0x00),
                Step::Write(0xA2),
                Step::Write(0x66),
                Step::Write(0x66),
                Step::Write(0x93),
                Step::Stop,
                Step::AdvanceUs(29_999),
            ],
            read_stream(3),
            vec![Step::AdvanceUs(1)],
            read_stream(3),
        ]),
    );
    assert_eq!(&got.bytes[..3], &[0xFF; 3], "not ready before 30 ms");
    assert_eq!(sensirion_words(&got.bytes[3..]).unwrap(), vec![31_234]);
}

#[test]
fn dht20_reports_busy_and_tracks_stimulus() {
    let mut d = dev("dht20", 0x38);
    send(&mut d, &[0xAC, 0x33, 0x00]);
    advance(&mut d, 79_999);
    assert_eq!(
        bare_read(&mut d, 1),
        vec![0x98],
        "BUSY | 0x18 during the 80 ms"
    );
    let got = run_i2c(
        &mut d,
        &script([
            vec![
                Step::Input("temperature", -20.0),
                Step::Input("humidity", 87.5),
            ],
            vec![
                Step::Start,
                Step::Write(0xAC),
                Step::Write(0x33),
                Step::Write(0x00),
                Step::Stop,
            ],
            vec![Step::AdvanceUs(80_000)],
            read_stream(7),
        ]),
    );
    let f = got.bytes;
    assert_eq!(f[6], crc8(&f[..6]));
    let raw = f[1..6].iter().fold(0u64, |a, &b| a << 8 | u64::from(b));
    assert_eq!(raw >> 20, (0.875f64 * 1_048_576.0).round() as u64);
    assert_eq!(
        raw & 0xFFFFF,
        (30.0f64 / 200.0 * 1_048_576.0).round() as u64
    );
}

#[test]
fn tsl2591_gates_on_aen_and_scales_with_gain_and_integration_time() {
    let mut d = dev("tsl2591", 0x29);
    // On a bus the device clock always runs; a test must start it, or the
    // engine's no-clock degradation reads every ready flag as set.
    advance(&mut d, 1);
    assert_eq!(rd(&mut d, 0xB4, 4), vec![0; 4], "channels read 0 until AEN");
    wr(&mut d, 0xA0, &[0x03]);
    assert_eq!(rd(&mut d, 0xB3, 1)[0] & 1, 0, "AVALID not yet set");
    // Medium gain (25×), 100 ms: 1200 × 25 = 30000.
    wr(&mut d, 0xA1, &[0x10]);
    assert_eq!(le16(&rd(&mut d, 0xB4, 2)), 30_000);
    // Medium gain, 200 ms: 60000.
    wr(&mut d, 0xA1, &[0x11]);
    assert_eq!(le16(&rd(&mut d, 0xB4, 2)), 60_000);
    // High gain (428×), 100 ms: saturates at the 100 ms maximum, 37888.
    wr(&mut d, 0xA1, &[0x20]);
    assert_eq!(le16(&rd(&mut d, 0xB4, 2)), 37_888);
    // …and at 65535 for any longer integration.
    wr(&mut d, 0xA1, &[0x25]);
    assert_eq!(le16(&rd(&mut d, 0xB4, 2)), 65_535);
    // SRESET self-clears; the gain/time bits read back.
    wr(&mut d, 0xA1, &[0x91]);
    assert_eq!(rd(&mut d, 0xA1, 1), vec![0x11]);
}

#[test]
fn ina226_current_and_power_follow_calibration() {
    let mut d = dev("ina226", 0x40);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)
    assert_eq!(be16(&rd(&mut d, 0x04, 2)), 0, "CURRENT with CAL = 0");
    assert_eq!(be16(&rd(&mut d, 0x03, 2)), 0, "POWER with CAL = 0");
    // CAL = 2048 ⇒ Current = Shunt × 2048 / 2048 = 10000.
    wr(&mut d, 0x05, &[0x08, 0x00]);
    assert_eq!(be16(&rd(&mut d, 0x04, 2)), 10_000);
    // Power = 10000 × 4000 / 20000 = 2000.
    assert_eq!(be16(&rd(&mut d, 0x03, 2)), 2_000);
    // CAL = 1024 halves the current LSB count.
    wr(&mut d, 0x05, &[0x04, 0x00]);
    assert_eq!(be16(&rd(&mut d, 0x04, 2)), 5_000);
    // Negative current: two's complement in SHUNT and CURRENT, |I| in POWER.
    run_i2c(&mut d, &[Step::Input("current", -0.25)]);
    assert_eq!(be16(&rd(&mut d, 0x01, 2)), (-10_000i16) as u16);
    assert_eq!(be16(&rd(&mut d, 0x04, 2)), (-5_000i16) as u16);
    assert_eq!(be16(&rd(&mut d, 0x03, 2)), 1_000);
    // Configuration write mask: D14:D12 stay 100, RST is not stored.
    wr(&mut d, 0x00, &[0x0D, 0x27]);
    assert_eq!(be16(&rd(&mut d, 0x00, 2)), 0x4D27);
    // CVRF rises 2.2 ms after that write, and clears when Mask/Enable is read.
    assert_eq!(be16(&rd(&mut d, 0x06, 2)) & 0x0008, 0);
    advance(&mut d, 2_200);
    assert_eq!(be16(&rd(&mut d, 0x06, 2)) & 0x0008, 0x0008);
    // RST: every register back to its power-on value.
    wr(&mut d, 0x00, &[0x80, 0x00]);
    assert_eq!(be16(&rd(&mut d, 0x00, 2)), 0x4127);
    assert_eq!(be16(&rd(&mut d, 0x05, 2)), 0);
}

#[test]
fn pmsa003i_repeats_whole_frames_and_checksums_what_it_sends() {
    let mut d = dev("pmsa003i", 0x12);
    let first = bare_read(&mut d, 32);
    let second = bare_read(&mut d, 32);
    assert_eq!(
        first, second,
        "a second bare read starts at the frame head again"
    );
    let got = run_i2c(
        &mut d,
        &script([
            vec![
                Step::Input("pm2_5", 123.4),
                Step::Input("pm10", 300.0),
                Step::Input("n0_3", 60_000.0),
            ],
            read_reg(0x00, 32),
        ]),
    );
    let f = got.bytes;
    assert_eq!(be16(&f[6..8]), 123, "PM2.5 CF=1");
    assert_eq!(be16(&f[12..14]), 123, "PM2.5 atmospheric");
    assert_eq!(be16(&f[14..16]), 300, "PM10 atmospheric");
    assert_eq!(be16(&f[16..18]), 60_000, ">0.3 µm count");
    let sum: u16 = f[..30].iter().map(|&b| u16::from(b)).sum();
    assert_eq!(be16(&f[30..32]), sum);
}

#[test]
fn icm42688_scales_saturates_and_banks() {
    let mut d = dev("icm42688", 0x68);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)
    assert_eq!(
        rd(&mut d, 0x2D, 1),
        vec![0x10],
        "INT_STATUS powers up RESET_DONE"
    );
    assert_eq!(rd(&mut d, 0x2D, 1), vec![0x00], "…and is clear-on-read");
    assert_eq!(
        be16(&rd(&mut d, 0x1D, 2)),
        0x8000,
        "temperature while every sensor is off"
    );
    wr(&mut d, 0x4E, &[0x0F]);
    assert_eq!(
        rd(&mut d, 0x2D, 1)[0] & 0x08,
        0,
        "DATA_RDY before one ODR period"
    );
    advance(&mut d, 1_000);
    assert_eq!(rd(&mut d, 0x2D, 1)[0] & 0x08, 0x08, "DATA_RDY after 1 ms");
    // 30 °C → 5 × 132.48 = 662.4 → 662.
    run_i2c(
        &mut d,
        &[
            Step::Input("temperature", 30.0),
            Step::Input("gyro_z", 100.0),
        ],
    );
    assert_eq!(be16(&rd(&mut d, 0x1D, 2)), 662);
    // 100 dps on ±2000 dps → 1640.
    assert_eq!(be16(&rd(&mut d, 0x29, 2)), 1_640);
    // ±2 g (FS_SEL = 3): 1 g → 16384; 3 g saturates at +32767.
    wr(&mut d, 0x50, &[0x66]);
    assert_eq!(be16(&rd(&mut d, 0x23, 2)), 16_384);
    run_i2c(&mut d, &[Step::Input("accel_z", 3.0)]);
    assert_eq!(be16(&rd(&mut d, 0x23, 2)), 0x7FFF);
    // Six data bytes in one burst (the common driver read).
    run_i2c(&mut d, &[Step::Input("accel_x", -0.5)]);
    let b = rd(&mut d, 0x1F, 6);
    assert_eq!(be16(&b[0..2]), (-8_192i16) as u16);
    // Accel low-power only: the gyro goes back to 0x8000.
    wr(&mut d, 0x4E, &[0x02]);
    assert_eq!(be16(&rd(&mut d, 0x29, 2)), 0x8000);
    // Another bank: WHO_AM_I's pointer decodes to nothing; bank 0 restores it.
    wr(&mut d, 0x76, &[0x01]);
    assert_eq!(rd(&mut d, 0x75, 1), vec![0x00]);
    wr(&mut d, 0x76, &[0x00]);
    assert_eq!(rd(&mut d, 0x75, 1), vec![0x47]);
}

#[test]
fn icm20948_banks_full_scale_and_temperature() {
    let mut d = dev("icm20948", 0x68);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)
    assert_eq!(be16(&rd(&mut d, 0x31, 2)), 0, "outputs read 0 while asleep");
    wr(&mut d, 0x06, &[0x01]);
    assert_eq!(
        rd(&mut d, 0x1A, 1)[0] & 1,
        0,
        "RAW_DATA_0_RDY not before one sample"
    );
    advance(&mut d, 1_000);
    assert_eq!(
        rd(&mut d, 0x1A, 1)[0] & 1,
        1,
        "RAW_DATA_0_RDY one sample after waking"
    );
    // Bank 2: the same pointer 0x00 is GYRO_SMPLRT_DIV, not WHO_AM_I.
    wr(&mut d, 0x7F, &[0x20]);
    assert_eq!(rd(&mut d, 0x00, 1), vec![0x00]);
    wr(&mut d, 0x14, &[0x06]); // ACCEL_FS_SEL = 3 (±16 g)
    wr(&mut d, 0x01, &[0x07]); // GYRO_FS_SEL = 3 (±2000 dps)
    wr(&mut d, 0x7F, &[0x00]);
    assert_eq!(rd(&mut d, 0x00, 1), vec![0xEA]);
    assert_eq!(be16(&rd(&mut d, 0x31, 2)), 2_048, "1 g at ±16 g");
    run_i2c(
        &mut d,
        &[
            Step::Input("gyro_x", -100.0),
            Step::Input("temperature", 30.0),
        ],
    );
    assert_eq!(be16(&rd(&mut d, 0x33, 2)), (-1_640i16) as u16);
    // (30 − 21) × 333.87 = 3004.83 → 3005.
    assert_eq!(be16(&rd(&mut d, 0x39, 2)), 3_005);
    // DEVICE_RESET puts the part back to sleep; outputs read 0 again.
    wr(&mut d, 0x06, &[0x80]);
    assert_eq!(rd(&mut d, 0x06, 1), vec![0x41]);
    assert_eq!(be16(&rd(&mut d, 0x31, 2)), 0);
}

#[test]
fn bme680_bosch_compensation_recovers_the_stimulus_exactly() {
    let mut d = dev("bme680", 0x76);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)
                        // Forced mode is required: without it new_data_0 never rises.
    assert_eq!(
        rd(&mut d, 0x1D, 1)[0] & 0x80,
        0,
        "new_data_0 before any measurement"
    );
    wr(&mut d, 0x74, &[0x25]);
    advance(&mut d, 7_255);
    assert_eq!(rd(&mut d, 0x1D, 1)[0] & 0x80, 0, "new_data_0 1 µs early");
    advance(&mut d, 1);
    assert_eq!(rd(&mut d, 0x1D, 1)[0] & 0x80, 0x80, "new_data_0 at 7256 µs");
    let cal = bosch::Calib::read(&mut d);
    let mut checked = 0;
    for &t in &[-40.0, -12.34, 0.0, 21.5, 25.0, 37.77, 60.0, 85.0] {
        // 533–600 hPa straddles the pres_ovf_check branch at these temperatures.
        for &p in &[
            300.0, 533.6, 560.01, 594.0, 594.5, 700.0, 950.0, 1013.25, 1050.0, 1100.0,
        ] {
            for &h in &[0.0, 0.5, 12.345, 50.0, 87.654, 100.0] {
                run_i2c(
                    &mut d,
                    &[
                        Step::Input("temperature", t),
                        Step::Input("pressure", p),
                        Step::Input("humidity", h),
                    ],
                );
                let r = bme680_forced(&mut d, &cal).unwrap();
                let at = format!("{t} °C / {p} hPa / {h} %RH");
                // Temperature: one ADC count moves Bosch's output by less than
                // its 0.01 °C unit, so it comes back EXACTLY.
                assert_eq!(r.t, (t * 100.0f64).round() as i16, "T at {at}");
                // Humidity: one count is ≈4 milli-%RH, so the requirement is
                // that the served ADC word is the NEAREST one — neither
                // neighbour lands closer to the stimulus.
                let want = (h * 1000.0f64).round() as i64;
                let err =
                    |adc: u16| (i64::from(bosch::calc_humidity(adc, r.t_fine, &cal)) - want).abs();
                let here = err(r.adc_h);
                assert!(
                    here <= err(r.adc_h.saturating_sub(1)) && here <= err(r.adc_h + 1),
                    "H at {at}: adc {} gives {} milli-%, a neighbour is closer",
                    r.adc_h,
                    r.h
                );
                assert!(here <= 3, "H at {at}: {} milli-% off", here);
                // Pressure: above Bosch's pres_ovf_check branch the integer
                // output moves in 2 Pa steps, so the requirement is the same
                // nearest-word property, within 1 Pa.
                let want_p = (p * 100.0f64).round() as i64;
                let perr = |adc: u32| {
                    (i64::from(bosch::calc_pressure(adc, r.t_fine, &cal)) - want_p).abs()
                };
                let here_p = perr(r.adc_p);
                assert!(
                    here_p <= perr(r.adc_p - 1) && here_p <= perr(r.adc_p + 1),
                    "P at {at}: adc {} gives {} Pa, a neighbour is closer",
                    r.adc_p,
                    r.p
                );
                assert!(here_p <= 1, "P at {at}: {} Pa off", here_p);
                checked += 1;
            }
        }
    }
    println!(
        "bme680: {checked} T/P/H points recovered through Bosch's integer compensation \
         (T exact, P ≤ 1 Pa, H ≤ 3 milli-%, every word the nearest)"
    );
    // The gas channel is not modelled and says so: gas_valid_r and
    // heat_stab_r are clear.
    let g = rd(&mut d, 0x2A, 2);
    assert_eq!(g[1] & 0x30, 0, "gas_valid_r / heat_stab_r must read clear");
    // Soft reset restores the control registers.
    wr(&mut d, 0x75, &[0x08]);
    wr(&mut d, 0xE0, &[0xB6]);
    assert_eq!(rd(&mut d, 0x75, 1), vec![0x00]);
    assert_eq!(
        rd(&mut d, 0xE0, 1),
        vec![0x00],
        "the reset register reads 0"
    );
}

// ─── batch 4: per-part behaviour beyond the default stimulus (I²C) ────────

/// The MCP23017 drives a pad only where IODIR made the pin an output, reads
/// the latch back on outputs and the posed level (IPOL-inverted) on inputs.
#[test]
fn mcp23017_tracks_stimulus() {
    use labwired_core::sim_input::SimInput;
    let mut d = dev("mcp23017", 0x20);
    assert!(d.take_pin_drives().is_empty(), "reset drives nothing");

    // §3.5.11: OLAT is written while every pin is still an input — no pad moves.
    wr(&mut d, 0x14, &[0xFF]);
    assert!(
        d.take_pin_drives().is_empty(),
        "OLATA written with IODIRA = 0xFF must not drive a pad"
    );
    // §3.5.1: GPA4..GPA7 become outputs and take their latch bits at once.
    wr(&mut d, 0x00, &[0x0F]);
    let mut drives = d.take_pin_drives();
    drives.sort();
    assert_eq!(
        drives,
        vec![
            ("GPA4".to_string(), true),
            ("GPA5".to_string(), true),
            ("GPA6".to_string(), true),
            ("GPA7".to_string(), true),
        ]
    );
    // GPIOA = latch on the outputs, the posed level on the inputs.
    d.set_input("port_a", 5.0).unwrap();
    assert_eq!(rd(&mut d, 0x12, 1), vec![0xF5]);
    // §3.5.2: IPOL inverts the input bits only.
    wr(&mut d, 0x02, &[0xFF]);
    assert_eq!(rd(&mut d, 0x12, 1), vec![0xFA]);

    // A 2-byte burst at GPIOA writes GPIOA then GPIOB, through to the latches.
    wr(&mut d, 0x01, &[0x00]); // IODIRB: all outputs
    let _ = d.take_pin_drives();
    wr(&mut d, 0x12, &[0x00, 0x81]);
    let mut drives = d.take_pin_drives();
    drives.sort();
    assert_eq!(
        drives,
        vec![
            ("GPA4".to_string(), false),
            ("GPA5".to_string(), false),
            ("GPA6".to_string(), false),
            ("GPA7".to_string(), false),
            ("GPB0".to_string(), true),
            ("GPB7".to_string(), true),
        ],
        "GPB1..GPB6 were first driven LOW by the IODIRB write (latch 0x00), so \
         only GPB0 and GPB7 move"
    );
    assert_eq!(rd(&mut d, 0x14, 2), vec![0x00, 0x81], "OLATA, OLATB");
    // Input pins of port A still read the posed level, inverted.
    assert_eq!(rd(&mut d, 0x12, 2), vec![0x0A, 0x81], "GPIOA, GPIOB");
}

/// The PCF8575 presents a word only when its pair completes, keeps
/// alternating halves on long reads, and a host-posed port reaches both the
/// wire and the pads.
#[test]
fn pcf8575_tracks_stimulus() {
    use labwired_core::sim_input::SimInput;
    let mut d = dev("pcf8575", 0x20);
    send(&mut d, &[0x0F, 0xF0]);
    let mut drives = d.take_pin_drives();
    drives.sort();
    let mut want: Vec<(String, bool)> = (0..8)
        .map(|n| (format!("P0{n}"), n < 4))
        .chain((0..8).map(|n| (format!("P1{n}"), n >= 4)))
        .collect();
    want.sort();
    assert_eq!(
        drives, want,
        "every pad takes its bit from the written pair"
    );

    // §8.1: the port moves only after the SECOND byte of a pair. An odd
    // trailing byte is a pair that never completed.
    send(&mut d, &[0x00]);
    assert!(d.take_pin_drives().is_empty(), "half a pair moved a pad");
    assert_eq!(bare_read(&mut d, 2), vec![0x0F, 0xF0]);
    // Three bytes: the first pair lands, the third byte does not.
    send(&mut d, &[0xAA, 0x55, 0x00]);
    assert_eq!(bare_read(&mut d, 2), vec![0xAA, 0x55]);
    // Two pairs in one transaction: "after every two bytes, the previous data
    // is overwritten".
    send(&mut d, &[0x11, 0x22, 0x33, 0x44]);
    assert_eq!(
        bare_read(&mut d, 6),
        vec![0x33, 0x44, 0x33, 0x44, 0x33, 0x44]
    );

    d.set_input("port", f64::from(0xBEEFu16)).unwrap();
    assert_eq!(bare_read(&mut d, 2), vec![0xEF, 0xBE]);
    let drives = d.take_pin_drives();
    assert!(
        drives.contains(&("P04".to_string(), false))
            && drives.contains(&("P16".to_string(), false)),
        "0xBEEF clears P04 and P16: {drives:?}"
    );
}

/// ADS1015 CONVERSION: 12-bit, left-justified, MUX-selected, PGA-scaled,
/// clipping at 7FFh / 800h (§7.3.3 Table 7-1, §7.5.4, Figure 8-2).
#[test]
fn ads1015_tracks_stimulus() {
    use labwired_core::sim_input::SimInput;
    // OS | MUX | PGA | MODE=1 | DR=100 | COMP_QUE=11.
    fn config(mux: u16, pga: u16) -> [u8; 2] {
        (0x8183 | mux << 12 | pga << 9).to_be_bytes()
    }
    fn conv(d: &mut GenericI2cDevice, mux: u16, pga: u16) -> u16 {
        wr(d, 0x01, &config(mux, pga));
        be16(&rd(d, 0x00, 2))
    }
    let mut d = dev("ads1015", 0x48);
    d.set_input("a0", 1.0004).unwrap();
    d.set_input("a1", 0.25).unwrap();
    d.set_input("a2", 0.1).unwrap();
    d.set_input("a3", -0.5).unwrap();
    // AIN0 at ±2.048 V: 1.0004 V → 1000.4 → 1000 counts → 1000 << 4. The
    // ADS1115 would report 16006 here; the low nibble is always zero.
    assert_eq!(conv(&mut d, 4, 2), 1000 << 4);
    // POR MUX 000 is AIN0 − AIN1: 0.7504 V → 750 counts.
    assert_eq!(conv(&mut d, 0, 2), 750 << 4);
    // AIN3 − GND, negative: −500 counts, two's complement in bits 15:4.
    assert_eq!(conv(&mut d, 7, 2), ((-500i16 as u16) & 0x0FFF) << 4);
    assert_eq!(conv(&mut d, 7, 2), 0xE0C0);
    // AIN2 at ±0.256 V (PGA 101/110/111): 0.1 V → 800 counts.
    for pga in 5..=7 {
        assert_eq!(conv(&mut d, 6, pga), 800 << 4, "PGA {pga:03b}");
    }
    // AIN0 at ±6.144 V: 1.0004 V × 2048 / 6.144 = 333.47 → 333.
    assert_eq!(conv(&mut d, 4, 0), 333 << 4);
    // AIN1 − AIN3 at ±1.024 V: 0.75 V → 1500.
    assert_eq!(conv(&mut d, 2, 3), 1500 << 4);
    // Clipping: +FS → 7FFh, −FS → 800h (§7.5.4).
    d.set_input("a0", 3.0).unwrap();
    assert_eq!(conv(&mut d, 4, 2), 0x7FF0);
    d.set_input("a0", -3.0).unwrap();
    assert_eq!(conv(&mut d, 4, 2), 0x8000);
}

// ─── batch 4: the SPI part (RFM95W) ────────────────────────────────────────
//
// The checks above talk I²C; the RFM95W is SPI. It gets the same shape — a
// check function, a smoke test, and a negative control against stand-ins
// answering only 0x00 or only 0xFF — over `run_spi`.

fn spi_dev(device_type: &str) -> GenericSpiDevice {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    GenericSpiDevice::from_yaml(yaml, "PA4").unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

/// One SX127x register access: address byte (bit 7 = write), one data byte.
/// Returns the MISO byte of the data phase.
fn sx(d: &mut dyn SpiDevice, addr: u8, data: u8) -> u8 {
    run_spi(d, &spi_xfer(&[addr, data])).bytes[1]
}

/// RFM95W / SX1276 (Table 41): RegVersion 0x12 (read-only), RegOpMode POR
/// 0x01, RegFrf POR 434 MHz, RegPaConfig 0x4F, RegPaDac 0x84; a write to
/// RegOpMode reads back.
fn check_rfm95(d: &mut dyn SpiDevice) -> Result<(), String> {
    ensure_eq!(sx(d, 0x42, 0x00), 0x12u8, "RFM95 RegVersion");
    ensure_eq!(
        sx(d, 0x01, 0x00),
        0x01u8,
        "RFM95 RegOpMode POR (FSK, standby)"
    );
    let frf = [sx(d, 0x06, 0), sx(d, 0x07, 0), sx(d, 0x08, 0)];
    ensure_eq!(frf, [0x6Cu8, 0x80, 0x00], "RFM95 RegFrf POR (434 MHz)");
    ensure_eq!(sx(d, 0x09, 0x00), 0x4Fu8, "RFM95 RegPaConfig POR");
    ensure_eq!(sx(d, 0x4D, 0x00), 0x84u8, "RFM95 RegPaDac POR");
    // LoRa mode, sleep — what every LoRa driver writes first.
    sx(d, 0x81, 0x80);
    ensure_eq!(sx(d, 0x01, 0x00), 0x80u8, "RFM95 RegOpMode after a write");
    Ok(())
}

type SpiCheck = fn(&mut dyn SpiDevice) -> Result<(), String>;

const SPI_PARTS: &[(&str, SpiCheck)] = &[("rfm95", check_rfm95)];

#[test]
fn every_spi_part_passes_its_smoke_check() {
    for &(ty, check) in SPI_PARTS {
        let mut d = spi_dev(ty);
        check(&mut d).unwrap_or_else(|e| panic!("{ty}: {e}"));
        println!("smoke  {ty:<9} (SPI): PASS");
    }
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
fn negative_control_a_silent_spi_part_fails_every_check() {
    for &(ty, check) in SPI_PARTS {
        for byte in [0x00u8, 0xFF] {
            match check(&mut SilentSpi { byte }) {
                Err(e) => {
                    println!("negctl {ty:<9} answering 0x{byte:02X}: FAILS as required ({e})")
                }
                Ok(()) => panic!(
                    "{ty}: a SPI part answering only 0x{byte:02X} PASSED the smoke check — \
                     the check proves nothing"
                ),
            }
        }
    }
}

/// The RFM95W has no measurement channel — its "stimulus" is the driver's own
/// configuration. A 915 MHz setup is stored and read back; RegVersion refuses
/// a write; one byte per frame (burst walk not modelled, see the descriptor).
#[test]
fn rfm95_tracks_stimulus() {
    let mut d = spi_dev("rfm95");
    // 915 MHz: Frf = 915e6 / (32e6 / 2^19) = 0xE4C000.
    for (addr, v) in [(0x06u8, 0xE4u8), (0x07, 0xC0), (0x08, 0x00)] {
        sx(&mut d, 0x80 | addr, v);
    }
    assert_eq!(
        [
            sx(&mut d, 0x06, 0),
            sx(&mut d, 0x07, 0),
            sx(&mut d, 0x08, 0)
        ],
        [0xE4, 0xC0, 0x00]
    );
    // PA_BOOST at +17 dBm (RegPaConfig 0x8F) and an OCP of 0x2B.
    sx(&mut d, 0x89, 0x8F);
    assert_eq!(sx(&mut d, 0x09, 0), 0x8F);
    // RegVersion is read-only.
    sx(&mut d, 0xC2, 0x00);
    assert_eq!(sx(&mut d, 0x42, 0), 0x12);
    // Untouched addresses keep their Table 41 POR values.
    assert_eq!(sx(&mut d, 0x0B, 0), 0x2B, "RegOcp");
    assert_eq!(sx(&mut d, 0x70, 0), 0xD0, "RegPll");
    assert_eq!(sx(&mut d, 0x3E, 0), 0x80, "RegIrqFlags1");
}

// ─── batch 4: the logic part (ULN2803A) ────────────────────────────────────
//
// A ULN2803A has no bus and no register: its "registers" are eight pads in and
// eight pads out. The check therefore runs over a `Pads` seam — the real part
// placed on a NUCLEO-L476RG bus, sampled through the GPIO input registers the
// CPU reads — and the negative control runs the SAME check against stand-ins
// whose outputs are stuck low, stuck high, or simply follow the inputs (a
// non-inverting buffer). All three must fail.

const ULN_IN: [&str; 8] = ["PC0", "PC1", "PC2", "PC3", "PC4", "PC5", "PC6", "PC7"];
const ULN_OUT: [&str; 8] = ["PB0", "PB1", "PB2", "PB3", "PB4", "PB5", "PB6", "PB7"];

trait Pads {
    /// Put `byte` on IN1..IN8 (bit 0 = IN1) and let the outputs settle.
    fn apply(&mut self, byte: u8);
    /// OUT1..OUT8 as the CPU samples them (bit 0 = OUT1).
    fn outputs(&mut self) -> u8;
}

struct UlnBench {
    bus: SystemBus,
    cycle: u64,
}

impl UlnBench {
    fn new(extra_config: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip_path = root.join("configs/chips/stm32l476.yaml");
        let chip = ChipDescriptor::from_file(&chip_path).expect("stm32l476 chip");
        let mut config = String::new();
        for n in 0..8 {
            config.push_str(&format!("      in{}_pin: \"{}\"\n", n + 1, ULN_IN[n]));
            config.push_str(&format!("      out{}_pin: \"{}\"\n", n + 1, ULN_OUT[n]));
        }
        config.push_str(extra_config);
        let yaml = format!(
            "name: \"uln2803-bench\"\nchip: \"{}\"\nboard_io: []\nexternal_devices:\n  \
             - id: \"u1\"\n    type: \"uln2803\"\n    connection: \"gpio\"\n    config:\n{config}",
            chip_path.display()
        );
        let manifest: SystemManifest = serde_yaml::from_str(&yaml).expect("manifest parses");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("attach uln2803");
        // RCC_AHB2ENR: clock GPIOA/B/C.
        bus.write_u32(0x4002_104C, 0b111)
            .expect("enable GPIO clocks");
        Self { bus, cycle: 0 }
    }

    fn drive(&mut self, pin: &str, high: bool) {
        let (addr, bit) = SystemBus::resolve_pin_odr_pub(&self.bus, pin).expect("ODR");
        let mut word = self.bus.read_u32(addr).expect("read ODR");
        if high {
            word |= 1 << bit;
        } else {
            word &= !(1 << bit);
        }
        self.bus.write_u32(addr, word).expect("write ODR");
    }

    fn sample(&mut self, pin: &str) -> bool {
        let (addr, bit) = SystemBus::resolve_pin_idr_pub(&self.bus, pin).expect("IDR");
        self.bus.read_u32(addr).expect("read IDR") >> bit & 1 != 0
    }

    fn tick(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.bus.set_current_cycle(self.cycle);
            let _ = self.bus.tick_peripherals_fully();
            self.cycle += 1;
        }
    }
}

impl Pads for UlnBench {
    fn apply(&mut self, byte: u8) {
        self.bus.set_current_cycle(self.cycle);
        for (n, pin) in ULN_IN.iter().enumerate() {
            self.drive(pin, byte >> n & 1 != 0);
        }
        // 200 ns is 16 cycles at the board's 80 MHz and 200 at the 1 GHz
        // placement below; give both plenty.
        self.tick(512);
    }
    fn outputs(&mut self) -> u8 {
        (0..8).fold(0u8, |acc, n| acc | (u8::from(self.sample(ULN_OUT[n])) << n))
    }
}

/// Stand-ins the check must reject.
enum FakeUln {
    Stuck(u8),
    Follower(u8),
}

impl Pads for FakeUln {
    fn apply(&mut self, byte: u8) {
        if let FakeUln::Follower(last) = self {
            *last = byte;
        }
    }
    fn outputs(&mut self) -> u8 {
        match self {
            FakeUln::Stuck(v) | FakeUln::Follower(v) => *v,
        }
    }
}

/// Every output is the complement of its input, channel by channel: all off,
/// all on, the two checkerboards and a walking one.
fn check_uln2803(p: &mut dyn Pads) -> Result<(), String> {
    let mut patterns = vec![0x00u8, 0xFF, 0xA5, 0x5A];
    patterns.extend((0..8).map(|n| 1u8 << n));
    for byte in patterns {
        p.apply(byte);
        ensure_eq!(
            p.outputs(),
            !byte,
            "ULN2803A OUT1..8 for IN1..8 = {byte:08b}"
        );
    }
    Ok(())
}

#[test]
fn uln2803_passes_its_smoke_check_on_l476_pads() {
    check_uln2803(&mut UlnBench::new("")).unwrap_or_else(|e| panic!("uln2803: {e}"));
    println!("smoke  uln2803   (GPIO): PASS");
}

#[test]
fn negative_control_a_stuck_or_non_inverting_uln2803_fails() {
    for (what, mut fake) in [
        ("stuck low", FakeUln::Stuck(0x00)),
        ("stuck high", FakeUln::Stuck(0xFF)),
        ("non-inverting buffer", FakeUln::Follower(0)),
    ] {
        match check_uln2803(&mut fake) {
            Err(e) => println!("negctl uln2803 {what}: FAILS as required ({e})"),
            Ok(()) => panic!("uln2803: a {what} stand-in PASSED the smoke check"),
        }
    }
}

/// The ULN2803A's stimulus is its input pads: a channel's output moves after
/// tOFF (200 ns) and not before. `cpu_hz: 1_000_000_000` makes one cycle one
/// nanosecond, so the deadline can be walked cycle by cycle.
#[test]
fn uln2803_tracks_stimulus() {
    let mut b = UlnBench::new("      cpu_hz: 1000000000\n");
    b.apply(0x00);
    assert_eq!(b.outputs(), 0xFF, "precondition: every channel off");
    let start = b.cycle;
    b.bus.set_current_cycle(start);
    b.drive(ULN_IN[2], true);
    assert!(
        b.sample(ULN_OUT[2]),
        "OUT3 moved inside the store that moved IN3"
    );
    b.tick(199);
    assert!(
        b.sample(ULN_OUT[2]),
        "OUT3 fell before the 200 ns propagation delay elapsed"
    );
    b.tick(2);
    assert!(
        !b.sample(ULN_OUT[2]),
        "OUT3 must sink once 200 ns have passed"
    );
    assert_eq!(b.outputs(), !0b0000_0100, "only channel 3 moved");
}
