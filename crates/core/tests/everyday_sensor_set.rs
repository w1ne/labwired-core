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
//! Batch 1b adds the MPU-9250 (accel/gyro/temperature; its AK8963
//! magnetometer is not modelled), the ENS160 (whose outputs are only live in
//! STANDARD mode), the INA260 (fixed-LSB current and power) and the INA3221
//! (13-bit words left-aligned by three bits). Two parts asked for in that
//! batch are absent on purpose, because the `i2c_device` primitive cannot
//! model them truthfully: the AM2320 (CRC-16 frames whose length follows the
//! request's register count) and the VL53L5CX (a firmware-upload boot sequence
//! whose registers no ST document publishes).
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

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

mod common;
use common::transcript::{read_reg, read_stream, run_i2c, script, write_reg, Step};

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

/// INA260: the ID registers, the power-on configuration, and the fixed-LSB
/// measurement words at 5 V / 1 A through the internal shunt: current 800
/// counts (1.25 mA), bus 4000 counts (1.25 mV), power 800 × 4000 / 6400 = 500
/// counts (10 mW) = 5 W.
fn check_ina260(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(be16(&rd(d, 0xFE, 2)), 0x5449u16, "INA260 manufacturer ID");
    ensure_eq!(be16(&rd(d, 0xFF, 2)), 0x2270u16, "INA260 die ID");
    ensure_eq!(
        be16(&rd(d, 0x00, 2)),
        0x6127u16,
        "INA260 configuration POR value"
    );
    ensure_eq!(
        be16(&rd(d, 0x01, 2)),
        800u16,
        "INA260 current (1.25 mA/LSB)"
    );
    ensure_eq!(
        be16(&rd(d, 0x02, 2)),
        4_000u16,
        "INA260 bus voltage (1.25 mV/LSB)"
    );
    ensure_eq!(be16(&rd(d, 0x03, 2)), 500u16, "INA260 power (10 mW/LSB)");
    Ok(())
}

/// INA3221: the ID registers, the power-on configuration and Mask/Enable (TCF
/// set), and channel 1 at 0.2 A through 0.1 Ω / 5 V: 500 shunt counts and 625
/// bus counts, each LEFT-ALIGNED by three bits.
fn check_ina3221(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(be16(&rd(d, 0xFE, 2)), 0x5449u16, "INA3221 manufacturer ID");
    ensure_eq!(be16(&rd(d, 0xFF, 2)), 0x3220u16, "INA3221 die ID");
    ensure_eq!(
        be16(&rd(d, 0x00, 2)),
        0x7127u16,
        "INA3221 configuration POR value"
    );
    // CVRF masked: with no device clock the engine reads data-ready as set.
    ensure_eq!(
        be16(&rd(d, 0x0F, 2)) & 0xFFFE,
        0x0002u16,
        "INA3221 Mask/Enable POR value (TCF)"
    );
    ensure_eq!(
        be16(&rd(d, 0x01, 2)),
        500u16 << 3,
        "INA3221 CH1 shunt voltage (40 µV/LSB, D2:D0 = 0)"
    );
    ensure_eq!(
        be16(&rd(d, 0x02, 2)),
        625u16 << 3,
        "INA3221 CH1 bus voltage (8 mV/LSB, D2:D0 = 0)"
    );
    ensure_eq!(
        be16(&rd(d, 0x06, 2)),
        1_500u16 << 3,
        "INA3221 CH3 bus at 12 V"
    );
    Ok(())
}

/// MPU-9250: WHO_AM_I, the power-on PWR_MGMT_1 (awake, unlike the MPU-6050),
/// and the standard 14-byte burst from 0x3B: accel Z at 1 g on ±2 g = 16384,
/// TEMP_OUT at 25 °C = 4 × 333.87 = 1335.48 → 1335, gyro at rest = 0.
fn check_mpu9250(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x75, 1), vec![0x71u8], "MPU-9250 WHO_AM_I");
    ensure_eq!(
        rd(d, 0x6B, 1),
        vec![0x01u8],
        "MPU-9250 PWR_MGMT_1 reset (CLKSEL = 1, awake)"
    );
    let b = rd(d, 0x3B, 14);
    ensure_eq!(be16(&b[0..2]), 0u16, "MPU-9250 accel X at rest");
    ensure_eq!(be16(&b[4..6]), 16_384u16, "MPU-9250 accel Z at 1 g, ±2 g");
    ensure_eq!(be16(&b[6..8]), 1_335u16, "MPU-9250 TEMP_OUT at 25 °C");
    ensure_eq!(be16(&b[8..10]), 0u16, "MPU-9250 gyro X at rest");
    Ok(())
}

/// ENS160: PART_ID (little-endian 0x0160), OPMODE powering up in IDLE, then
/// STANDARD mode and the AQI / TVOC / eCO2 burst from 0x21 at the default
/// stimulus (AQI 2, 100 ppb, 600 ppm), with STATAS set and VALIDITY normal.
fn check_ens160(d: &mut dyn I2cDevice) -> Result<(), String> {
    ensure_eq!(rd(d, 0x00, 2), vec![0x60u8, 0x01], "ENS160 PART_ID");
    ensure_eq!(rd(d, 0x10, 1), vec![0x01u8], "ENS160 OPMODE reset (IDLE)");
    wr(d, 0x10, &[0x02]); // STANDARD
    ensure_eq!(
        rd(d, 0x20, 1)[0] & 0x8C,
        0x80u8,
        "ENS160 DEVICE_STATUS: STATAS, VALIDITY = normal"
    );
    let b = rd(d, 0x21, 5);
    ensure_eq!(b[0], 2u8, "ENS160 DATA_AQI");
    ensure_eq!(le16(&b[1..3]), 100u16, "ENS160 DATA_TVOC (ppb)");
    ensure_eq!(le16(&b[3..5]), 600u16, "ENS160 DATA_ECO2 (ppm)");
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
    ("mpu9250", 0x68, check_mpu9250),
    ("ens160", 0x52, check_ens160),
    ("ina260", 0x40, check_ina260),
    ("ina3221", 0x40, check_ina3221),
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
fn ina260_tracks_stimulus_with_fixed_lsbs() {
    let mut d = dev("ina260", 0x40);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)

    // SBOS656 Table 1, the datasheet's own worked example: 12.5 A at 11.98 V.
    run_i2c(
        &mut d,
        &[
            Step::Input("current", 12.5),
            Step::Input("bus_voltage", 11.98),
        ],
    );
    assert_eq!(be16(&rd(&mut d, 0x01, 2)), 0x2710, "Current 10000 counts");
    assert_eq!(
        be16(&rd(&mut d, 0x02, 2)),
        0x2570,
        "Bus voltage 9584 counts"
    );
    assert_eq!(be16(&rd(&mut d, 0x03, 2)), 0x3A7F, "Power 14975 counts");
    // Negative current: two's complement in CURRENT, |I| in POWER.
    run_i2c(&mut d, &[Step::Input("current", -2.5)]);
    assert_eq!(be16(&rd(&mut d, 0x01, 2)), (-2_000i16) as u16);
    // 2000 × 9584 / 6400 = 2995.
    assert_eq!(be16(&rd(&mut d, 0x03, 2)), 2_995);
    // Configuration write mask: D14:D12 stay 110, RST is not stored.
    wr(&mut d, 0x00, &[0x0D, 0x27]);
    assert_eq!(be16(&rd(&mut d, 0x00, 2)), 0x6D27);
    // CVRF rises 2.2 ms after that write, and clears when Mask/Enable is read.
    assert_eq!(be16(&rd(&mut d, 0x06, 2)) & 0x0008, 0);
    advance(&mut d, 2_200);
    assert_eq!(be16(&rd(&mut d, 0x06, 2)) & 0x0008, 0x0008);
    assert_eq!(
        be16(&rd(&mut d, 0x06, 2)) & 0x0008,
        0,
        "cleared by the read"
    );
    // Mask/Enable: the flag bits are the part's.
    wr(&mut d, 0x06, &[0xFF, 0xFF]);
    assert_eq!(be16(&rd(&mut d, 0x06, 2)) & 0xFFF7, 0xFC03);
    // RST: back to power-on values.
    wr(&mut d, 0x00, &[0x80, 0x00]);
    assert_eq!(be16(&rd(&mut d, 0x00, 2)), 0x6127);
    assert_eq!(be16(&rd(&mut d, 0x06, 2)), 0);
}

#[test]
fn ina3221_left_aligns_saturates_and_sums() {
    let mut d = dev("ina3221", 0x40);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)

    // SBOS576 §8.2.2's worked example: −80 mV → 2000 counts → 0xC180. At
    // 0.1 Ω that is −0.8 A.
    run_i2c(&mut d, &[Step::Input("ch1_current", -0.8)]);
    assert_eq!(be16(&rd(&mut d, 0x01, 2)), 0xC180);
    // Rounded to a whole count BEFORE the shift: 0.12345 A → 308.625 → 309
    // counts → 2472, and D2:D0 are zero.
    run_i2c(&mut d, &[Step::Input("ch2_current", 0.12345)]);
    let w = be16(&rd(&mut d, 0x03, 2));
    assert_eq!(w, 309 << 3);
    assert_eq!(w & 0x7, 0);
    // Shunt full scale, 163.8 mV, is 0x7FF8 (§8.2.2); 2 A × 0.1 Ω saturates.
    run_i2c(&mut d, &[Step::Input("ch3_current", 2.0)]);
    assert_eq!(be16(&rd(&mut d, 0x05, 2)), 0x7FF8);
    run_i2c(&mut d, &[Step::Input("ch3_current", -2.0)]);
    assert_eq!(be16(&rd(&mut d, 0x05, 2)), 0x8000);
    // Bus 12.0 V → 1500 counts → 0x2EE0.
    run_i2c(&mut d, &[Step::Input("ch3_bus_voltage", 12.0)]);
    assert_eq!(be16(&rd(&mut d, 0x06, 2)), 0x2EE0);
    // Shunt-voltage sum: 0 until SCC is set, then Σ counts in D14:D1.
    assert_eq!(be16(&rd(&mut d, 0x0D, 2)), 0);
    run_i2c(
        &mut d,
        &[
            Step::Input("ch1_current", 0.2), // 500
            Step::Input("ch2_current", 0.1), // 250
            Step::Input("ch3_current", 0.5), // 1250
        ],
    );
    wr(&mut d, 0x0F, &[0x70, 0x00]); // SCC1 | SCC2 | SCC3
    assert_eq!(be16(&rd(&mut d, 0x0F, 2)), 0x7002, "SCC stored, TCF kept");
    assert_eq!(be16(&rd(&mut d, 0x0D, 2)), 2_000 << 1);
    wr(&mut d, 0x0F, &[0x40, 0x00]); // SCC1 only
    assert_eq!(be16(&rd(&mut d, 0x0D, 2)), 500 << 1);
    // A negative channel sums as two's complement.
    run_i2c(&mut d, &[Step::Input("ch1_current", -0.2)]);
    assert_eq!(be16(&rd(&mut d, 0x0D, 2)), (-1_000i16) as u16);
    // CVRF: three channels × (1.1 ms + 1.1 ms) = 6.6 ms after a Configuration
    // write, cleared by reading Mask/Enable.
    wr(&mut d, 0x00, &[0x71, 0x27]);
    assert_eq!(be16(&rd(&mut d, 0x0F, 2)) & 1, 0);
    advance(&mut d, 6_600);
    assert_eq!(be16(&rd(&mut d, 0x0F, 2)) & 1, 1);
    assert_eq!(be16(&rd(&mut d, 0x0F, 2)) & 1, 0, "cleared by the read");
    // Limit registers: reserved D2:D0 read 0; RST restores the POR values.
    wr(&mut d, 0x07, &[0x12, 0x3F]);
    assert_eq!(be16(&rd(&mut d, 0x07, 2)), 0x1238);
    wr(&mut d, 0x00, &[0x80, 0x00]);
    assert_eq!(be16(&rd(&mut d, 0x00, 2)), 0x7127);
    assert_eq!(be16(&rd(&mut d, 0x07, 2)), 0x7FF8);
    assert_eq!(be16(&rd(&mut d, 0x0F, 2)), 0x0002);
    assert_eq!(be16(&rd(&mut d, 0x10, 2)), 0x2710);
    assert_eq!(be16(&rd(&mut d, 0x11, 2)), 0x2328);
}

#[test]
fn mpu9250_scales_tracks_data_ready_and_resets() {
    let mut d = dev("mpu9250", 0x68);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)
    assert_eq!(
        rd(&mut d, 0x3A, 1)[0] & 1,
        0,
        "RAW_DATA_RDY before a sample"
    );
    advance(&mut d, 1_000);
    assert_eq!(rd(&mut d, 0x3A, 1)[0] & 1, 1, "RAW_DATA_RDY after 1 ms");
    assert_eq!(
        rd(&mut d, 0x3A, 1)[0] & 1,
        0,
        "…cleared by reading INT_STATUS"
    );
    // ±16 g (ACCEL_FS_SEL = 3): 1 g → 2048; −3 g → −6144.
    wr(&mut d, 0x1C, &[0x18]);
    assert_eq!(be16(&rd(&mut d, 0x3F, 2)), 2_048);
    run_i2c(&mut d, &[Step::Input("accel_z", -3.0)]);
    assert_eq!(be16(&rd(&mut d, 0x3F, 2)), (-6_144i16) as u16);
    // Reserved ACCEL_CONFIG bits 2:0 do not store.
    wr(&mut d, 0x1C, &[0xFF]);
    assert_eq!(rd(&mut d, 0x1C, 1), vec![0xF8]);
    // ±500 dps (GYRO_FS_SEL = 1): 100 dps → 6550; −250 dps → −16375.
    wr(&mut d, 0x1B, &[0x08]);
    run_i2c(
        &mut d,
        &[Step::Input("gyro_x", 100.0), Step::Input("gyro_y", -250.0)],
    );
    let g = rd(&mut d, 0x43, 4);
    assert_eq!(be16(&g[0..2]), 6_550);
    assert_eq!(be16(&g[2..4]), (-16_375i16) as u16);
    // TEMP_OUT = (T − 21) × 333.87: 30 °C → 3004.83 → 3005; −10 °C → −10350.
    run_i2c(&mut d, &[Step::Input("temperature", 30.0)]);
    assert_eq!(be16(&rd(&mut d, 0x41, 2)), 3_005);
    run_i2c(&mut d, &[Step::Input("temperature", -10.0)]);
    assert_eq!(be16(&rd(&mut d, 0x41, 2)), (-10_350i16) as u16);
    // H_RESET: PWR_MGMT_1 back to 0x01 and the full-scale selects to ±2 g /
    // ±250 dps.
    wr(&mut d, 0x6B, &[0x80]);
    assert_eq!(rd(&mut d, 0x6B, 1), vec![0x01]);
    assert_eq!(rd(&mut d, 0x1C, 1), vec![0x00]);
    assert_eq!(rd(&mut d, 0x1B, 1), vec![0x00]);
    // −3 g on ±2 g saturates at the end of the int16.
    assert_eq!(be16(&rd(&mut d, 0x3F, 2)), 0x8000);
    assert_eq!(be16(&rd(&mut d, 0x43, 2)), 13_100, "100 dps at ±250 dps");
}

#[test]
fn ens160_gates_on_standard_mode_and_tracks_stimulus() {
    let mut d = dev("ens160", 0x52);
    advance(&mut d, 1); // start the device clock (see the TSL2591 test)

    // IDLE: no measurement yet; DATA_x hold their datasheet defaults.
    assert_eq!(rd(&mut d, 0x20, 1), vec![0x00], "DEVICE_STATUS in IDLE");
    assert_eq!(
        rd(&mut d, 0x21, 5),
        vec![0x01, 0, 0, 0, 0],
        "AQI 1, TVOC 0, eCO2 0"
    );
    assert_eq!(le16(&rd(&mut d, 0x30, 2)), 0x4A8A, "DATA_T default (25 °C)");
    assert_eq!(
        le16(&rd(&mut d, 0x32, 2)),
        0x6400,
        "DATA_RH default (50 %RH)"
    );
    // STANDARD: STATAS and NEWDAT rise, the outputs follow the stimulus.
    wr(&mut d, 0x10, &[0x02]);
    assert_eq!(rd(&mut d, 0x20, 1)[0] & 0x82, 0x82, "STATAS | NEWDAT");
    run_i2c(
        &mut d,
        &[
            Step::Input("eco2", 1_234.7),
            Step::Input("tvoc", 65_000.0),
            Step::Input("aqi", 4.0),
        ],
    );
    let b = rd(&mut d, 0x21, 5);
    assert_eq!(b[0], 4);
    assert_eq!(le16(&b[1..3]), 65_000, "TVOC at its 65000 ppb ceiling");
    assert_eq!(le16(&b[3..5]), 1_234, "eCO2 truncates to whole ppm");
    // Outside the datasheet's output range the host cannot pose a value.
    assert!(d.set_input("eco2", 100.0).is_err(), "eCO2 below 400 ppm");
    // The validity flag is the host's: warm-up reads VALIDITY = 1.
    run_i2c(&mut d, &[Step::Input("validity", 1.0)]);
    assert_eq!(rd(&mut d, 0x20, 1)[0] & 0x0C, 0x04);
    // Back to IDLE: STATAS clears and the last result is held.
    wr(&mut d, 0x10, &[0x01]);
    run_i2c(&mut d, &[Step::Input("eco2", 900.0)]);
    assert_eq!(rd(&mut d, 0x20, 1)[0] & 0x80, 0);
    assert_eq!(le16(&rd(&mut d, 0x24, 2)), 1_234);
    // TEMP_IN (Kelvin × 64, LSB first) is what DATA_T reports. 30 °C → 0x4BCA.
    wr(&mut d, 0x13, &[0xCA, 0x4B]);
    assert_eq!(rd(&mut d, 0x13, 2), vec![0xCA, 0x4B]);
    assert_eq!(le16(&rd(&mut d, 0x30, 2)), 0x4BCA);
    // RH_IN → DATA_RH: 40 %RH × 512 = 0x5000.
    wr(&mut d, 0x15, &[0x00, 0x50]);
    assert_eq!(le16(&rd(&mut d, 0x32, 2)), 0x5000);
    // RESET returns to IDLE.
    wr(&mut d, 0x10, &[0x02]);
    wr(&mut d, 0x10, &[0xF0]);
    assert_eq!(rd(&mut d, 0x10, 1), vec![0x01]);
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
