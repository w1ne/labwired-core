// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Byte-parity ratchet for every shipping declarative device.**
//!
//! `veml7700_parity.rs` proves ONE device byte-identical against a hand-written
//! oracle. That template does not cover the risk this file exists for: a change
//! to the SHARED declarative engine (`declarative_i2c.rs`, `declarative_spi.rs`,
//! `declarative_regs.rs`) silently altering a device that was never touched.
//! There is no hand-written oracle left for most of them, so the oracle here is
//! the engine's own output from BEFORE the change: each device is driven through
//! a fixed I²C / SPI script and its complete byte transcript is pinned as a
//! golden constant.
//!
//! The goldens below were captured on the pristine tree at `84456f0d` (the
//! parent commit of the `data_ready` primitive) by running this file with
//! `-- --nocapture` on a `git stash`ed working tree, and are asserted unchanged
//! after it. A future engine change that moves any of these bytes has changed a
//! part's behaviour and must say so out loud.
//!
//! Scripts are deliberately protocol-shaped, not exhaustive: they exercise the
//! paths a shared-engine change is most likely to disturb — rw write
//! accumulation and read-back (the `write_mask` path), pointer masking,
//! self-driving `updates`, `delay_us` gating, CRC framing, register-file
//! auto-increment, and SPI burst auto-increment.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::sim_input::SimInput;

// ─── device construction ───────────────────────────────────────────────────

fn i2c_device(ty: &str) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml(ty)
        .unwrap_or_else(|| panic!("{ty} descriptor is embedded"));
    GenericI2cDevice::from_yaml(yaml, 0).unwrap_or_else(|e| panic!("{ty}.yaml must parse: {e}"))
}

fn spi_device(ty: &str) -> GenericSpiDevice {
    let yaml = labwired_config::embedded_device_yaml(ty)
        .unwrap_or_else(|| panic!("{ty} descriptor is embedded"));
    GenericSpiDevice::from_yaml(yaml, "CS").unwrap_or_else(|e| panic!("{ty}.yaml must parse: {e}"))
}

// ─── script helpers ────────────────────────────────────────────────────────

/// Point at `reg`, repeated-START into the read phase, read `n` bytes.
fn read_reg(d: &mut dyn I2cDevice, reg: u8, n: usize) -> Vec<u8> {
    d.start();
    d.write(reg);
    d.start();
    let out: Vec<u8> = (0..n).map(|_| d.read()).collect();
    d.stop();
    out
}

/// Write `bytes` into `reg` (pointer first), framed START … STOP.
fn write_reg(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for &b in bytes {
        d.write(b);
    }
    d.stop();
}

/// Send a 16-bit big-endian opcode.
fn send_cmd16(d: &mut dyn I2cDevice, code: u16) {
    d.start();
    d.write((code >> 8) as u8);
    d.write((code & 0xFF) as u8);
    d.stop();
}

/// Send a single-byte opcode.
fn send_cmd8(d: &mut dyn I2cDevice, code: u8) {
    d.start();
    d.write(code);
    d.stop();
}

/// Read `n` bytes from a fresh read phase (command devices have no pointer).
fn read_stream(d: &mut dyn I2cDevice, n: usize) -> Vec<u8> {
    d.start();
    let out: Vec<u8> = (0..n).map(|_| d.read()).collect();
    d.stop();
    out
}

/// Clock `mosi` through a full CS-framed SPI transfer, collecting MISO.
fn spi_xfer(d: &mut dyn SpiDevice, mosi: &[u8]) -> Vec<u8> {
    d.cs_select();
    let out: Vec<u8> = mosi.iter().map(|&b| d.transfer(b)).collect();
    d.cs_release();
    out
}

/// Render a transcript as the literal that belongs in the golden table, so a
/// deliberate change is a copy-paste rather than a hand edit.
fn show(name: &str, bytes: &[u8]) {
    let hex: Vec<String> = bytes.iter().map(|b| format!("0x{b:02X}")).collect();
    println!("{name}: &[{}],", hex.join(", "));
}

/// Assert a transcript against its golden, printing it either way so a
/// `--nocapture` run of this file regenerates the whole table.
fn check(name: &str, got: &[u8], want: &[u8]) {
    show(name, got);
    assert_eq!(
        got, want,
        "{name}: the shared declarative engine changed this device's bytes"
    );
}

// ─── TMP102: register-pointer, pointer_mask, self-driving updates ───────────

#[test]
fn tmp102_transcript_is_unchanged() {
    let mut d = i2c_device("tmp102");
    let mut t = Vec::new();
    // Power-on registers, then four consecutive TEMP reads (each fires the
    // +0.5 °C add_wrap update), then the aliasing pointer (0x05 & 0x03 == 0x01).
    t.extend(read_reg(&mut d, 0x01, 2));
    t.extend(read_reg(&mut d, 0x02, 2));
    t.extend(read_reg(&mut d, 0x03, 2));
    for _ in 0..4 {
        t.extend(read_reg(&mut d, 0x00, 2));
    }
    t.extend(read_reg(&mut d, 0x05, 2));
    // TEMP is read-only: a write must be absorbed, not stored.
    write_reg(&mut d, 0x00, &[0xDE, 0xAD]);
    t.extend(read_reg(&mut d, 0x00, 2));
    // A short read (one byte of a two-byte word) must not fire the update.
    t.extend(read_reg(&mut d, 0x00, 1));
    t.extend(read_reg(&mut d, 0x00, 2));
    check("tmp102", &t, TMP102_GOLDEN);
}

const TMP102_GOLDEN: &[u8] = &[
    0x60, 0xA0, 0x4B, 0x00, 0x50, 0x00, 0x19, 0x00, 0x19, 0x80, 0x1A, 0x00, 0x1A, 0x80, 0x60, 0xA0,
    0x1B, 0x00, 0x1B, 0x1B, 0x80,
];

// ─── VEML7700: register-pointer, rw round-trip, resolution + scale_from ────

#[test]
fn veml7700_transcript_is_unchanged() {
    let mut d = i2c_device("veml7700");
    let mut t = Vec::new();
    // Power-on ALS/WHITE. The part boots SHUT DOWN (ALS_CONF resets to 0x0001,
    // ALS_SD set — datasheet Rev. 1.8 p7), so both read 0x0000 despite the
    // default 450 lux scene. The four bytes below used to be 0x85,0x1E /
    // 0x18,0x23 — a light reading from a sensor that had never been powered on.
    t.extend(read_reg(&mut d, 0x04, 2));
    t.extend(read_reg(&mut d, 0x05, 2));
    // Program every rw register and read each back — the write path this
    // change touched. ALS_CONF = gain ×2 (bits 12:11 = 01), IT 200 ms. Bit 0
    // (ALS_SD) is clear in 0x0840, so this write also POWERS THE PART ON, which
    // is why every byte from here on is unchanged by the shutdown gate.
    write_reg(&mut d, 0x00, &[0x40, 0x08]);
    write_reg(&mut d, 0x01, &[0x34, 0x12]);
    write_reg(&mut d, 0x02, &[0x78, 0x56]);
    write_reg(&mut d, 0x03, &[0x03, 0x00]);
    for reg in [0x00u8, 0x01, 0x02, 0x03] {
        t.extend(read_reg(&mut d, reg, 2));
    }
    // The reprogrammed resolution must change the counts.
    t.extend(read_reg(&mut d, 0x04, 2));
    t.extend(read_reg(&mut d, 0x05, 2));
    // Read-only ALS_INT, and an undeclared pointer (zero word).
    t.extend(read_reg(&mut d, 0x06, 2));
    t.extend(read_reg(&mut d, 0x7E, 2));
    // Drive the measurement channel and re-read.
    d.set_input("lux", 13.5).unwrap();
    t.extend(read_reg(&mut d, 0x04, 2));
    t.extend(read_reg(&mut d, 0x05, 2));
    check("veml7700", &t, VEML7700_GOLDEN);
}

/// Regenerated (via this file's own `--nocapture` self-print, never hand-typed)
/// when the VEML7700 gained its documented power-on shutdown state.
///
/// EXACTLY four bytes moved, all in the leading power-on read:
///   `0x85, 0x1E` → `0x00, 0x00`   ALS   at power-on
///   `0x18, 0x23` → `0x00, 0x00`   WHITE at power-on
/// The remaining 20 bytes are byte-identical, because the script's first write
/// (ALS_CONF = 0x0840) clears ALS_SD and powers the part on.
///
/// Those four bytes were a bug, not a behaviour change: the model was reporting
/// 450 lx from a sensor whose shutdown bit had never been cleared.
const VEML7700_GOLDEN: &[u8] = &[
    0x00, 0x00, 0x00, 0x00, 0x40, 0x08, 0x34, 0x12, 0x78, 0x56, 0x03, 0x00, 0x12, 0x7A, 0x62, 0x8C,
    0x00, 0x00, 0x00, 0x00, 0xAA, 0x03, 0x36, 0x04,
];

// ─── BH1750: single-byte opcodes, no CRC ───────────────────────────────────

#[test]
fn bh1750_transcript_is_unchanged() {
    let mut d = i2c_device("bh1750");
    let mut t = Vec::new();
    // Write-only opcodes queue nothing (reads must be 0xFF).
    send_cmd8(&mut d, 0x01); // power_on
    t.extend(read_stream(&mut d, 2));
    send_cmd8(&mut d, 0x07); // reset
    t.extend(read_stream(&mut d, 2));
    for code in [0x10u8, 0x11, 0x13, 0x20, 0x21, 0x23] {
        send_cmd8(&mut d, code);
        t.extend(read_stream(&mut d, 2));
    }
    send_cmd8(&mut d, 0xAB); // unknown opcode
    t.extend(read_stream(&mut d, 2));
    d.set_input("lux", 1234.0).unwrap();
    send_cmd8(&mut d, 0x10);
    t.extend(read_stream(&mut d, 3)); // one byte past the response
    check("bh1750", &t, BH1750_GOLDEN);
}

const BH1750_GOLDEN: &[u8] = &[
    0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x1C, 0x02, 0x1C, 0x02, 0x1C, 0x02, 0x1C, 0x02, 0x1C, 0x02, 0x1C,
    0xFF, 0xFF, 0x05, 0xC9, 0xFF,
];

// ─── SHT31: 16-bit opcodes, CRC-8 framing, delay_us gating ─────────────────

#[test]
fn sht31_transcript_is_unchanged() {
    let mut d = i2c_device("sht31");
    let mut t = Vec::new();
    send_cmd16(&mut d, 0xF32D); // read_status, no delay
    t.extend(read_stream(&mut d, 3));
    send_cmd16(&mut d, 0x30A2); // soft_reset, write-only
    t.extend(read_stream(&mut d, 3));
    // A 15 ms delayed measurement: not ready, still not ready one µs short,
    // then ready. This is the `advance_time_us` path the primitive shares.
    send_cmd16(&mut d, 0x2400);
    t.extend(read_stream(&mut d, 6));
    d.advance_time_us(14_999);
    t.extend(read_stream(&mut d, 6));
    d.advance_time_us(1);
    t.extend(read_stream(&mut d, 6));
    d.set_input("temperature", -12.5).unwrap();
    d.set_input("humidity", 88.0).unwrap();
    send_cmd16(&mut d, 0x2C06);
    d.advance_time_us(20_000);
    t.extend(read_stream(&mut d, 6));
    check("sht31", &t, SHT31_GOLDEN);
}

const SHT31_GOLDEN: &[u8] = &[
    0x80, 0x10, 0xE1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0x62, 0x03, 0x5E, 0x73, 0x33, 0x01, 0x2F, 0x8B, 0x54, 0xE1, 0x47, 0xA9,
];

// ─── PCA9685: byte-addressable register file, MODE1.AI auto-increment ──────

#[test]
fn pca9685_transcript_is_unchanged() {
    let mut d = i2c_device("pca9685");
    let mut t = Vec::new();
    // Power-on MODE1 (0x11 = SLEEP | ALLCALL), read without auto-increment:
    // the pointer must NOT walk, so both bytes are MODE1.
    d.start();
    d.write(0x00);
    d.start();
    t.push(d.read());
    t.push(d.read());
    d.stop();
    // The rest of the documented power-on state (datasheet Rev. 4), still with
    // auto-increment off so each read re-selects its pointer. Pinned here so a
    // shared-engine change that drops the register-file `reset` map shows up in
    // this transcript, not just in the PCA9685-specific parity file.
    for reg in [
        0x01u8, // MODE2      0x04  OUTDRV
        0x02,   // SUBADR1    0xE2
        0x05,   // ALLCALLADR 0xE0
        0x09,   // LED0_OFF_H 0x10  full OFF
        0xFB,   // ALL_LED_ON_H  — 0x00 on purpose (vendor contradicts itself)
        0xFE,   // PRE_SCALE  0x1E  200 Hz
    ] {
        d.start();
        d.write(reg);
        d.start();
        t.push(d.read());
        d.stop();
    }
    // Enable AI, then block-write channel 0's four LED registers in one frame.
    write_reg(&mut d, 0x00, &[0x21]);
    write_reg(&mut d, 0x06, &[0x00, 0x00, 0x29, 0x01]);
    // Read the block back — the pointer walks now.
    d.start();
    d.write(0x06);
    d.start();
    for _ in 0..4 {
        t.push(d.read());
    }
    d.stop();
    check("pca9685", &t, PCA9685_GOLDEN);
    // The observable derived from the file (engineering units, not bytes) —
    // OFF = 0x129 (297 counts) is the ~90° servo position.
    let angle = d.observable("servo_angle", 0).expect("channel 0 written");
    assert!(
        (angle - 90.0).abs() < 0.5,
        "servo_angle drifted: {angle}, expected ~90°"
    );
}

/// Regenerated (via this file's own `--nocapture` self-print, never hand-typed)
/// when the PCA9685 gained its documented power-on register values.
///
/// The OLD golden was `[0x11, 0x11, 0x00, 0x00, 0x29, 0x01]`. Note what did and
/// did not move:
///   * `0x11, 0x11` — MODE1 read twice, pointer parked. UNCHANGED.
///   * `0x04, 0xE2, 0xE0, 0x10, 0x00, 0x1E` — SIX NEW BYTES, from six reads
///     added to the script above, not from any byte changing value. They pin
///     MODE2 / SUBADR1 / ALLCALLADR / LED0_OFF_H / ALL_LED_ON_H / PRE_SCALE.
///   * `0x00, 0x00, 0x29, 0x01` — the channel-0 block read back after the
///     script writes it. UNCHANGED, and it could not have changed: the script
///     writes all four of those registers (0x06..0x09) before reading them, so
///     LED0_OFF_H's new 0x10 reset is overwritten by 0x01 first.
///
/// That last point is why the old transcript was blind to this bug, and why the
/// six reads were added rather than the golden simply re-blessed.
const PCA9685_GOLDEN: &[u8] = &[
    0x11, 0x11, 0x04, 0xE2, 0xE0, 0x10, 0x00, 0x1E, 0x00, 0x00, 0x29, 0x01,
];

// ─── ADXL345 (SPI): rw registers, burst auto-increment, signed words ───────

#[test]
fn adxl345_spi_transcript_is_unchanged() {
    let mut d = spi_device("adxl345_spi");
    let mut t = Vec::new();
    // DEVID read (bit 7 = read).
    t.extend(spi_xfer(&mut d, &[0x80, 0x00]));
    // Write POWER_CTL = 0x08 and DATA_FORMAT = 0x0B, then read both back.
    spi_xfer(&mut d, &[0x2D, 0x08]);
    spi_xfer(&mut d, &[0x31, 0x0B]);
    t.extend(spi_xfer(&mut d, &[0xAD, 0x00]));
    t.extend(spi_xfer(&mut d, &[0xB1, 0x00]));
    // Six-byte burst read from DATAX0 (0x32 | read | multi-byte).
    t.extend(spi_xfer(&mut d, &[0xF2, 0, 0, 0, 0, 0, 0]));
    // Negative g on X, positive on Y — two's complement little-endian.
    d.set_input("accel_x", -0.5).unwrap();
    d.set_input("accel_y", 0.25).unwrap();
    t.extend(spi_xfer(&mut d, &[0xF2, 0, 0, 0, 0, 0, 0]));
    check("adxl345_spi", &t, ADXL345_GOLDEN);
}

const ADXL345_GOLDEN: &[u8] = &[
    0x00, 0xE5, 0x00, 0x08, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x80, 0xFF,
    0x40, 0x00, 0x00, 0x01,
];

// ─── MAX31855 (SPI): command-less 32-bit composite frame ───────────────────

#[test]
fn max31855_transcript_is_unchanged() {
    let mut d = spi_device("max31855");
    let mut t = Vec::new();
    t.extend(spi_xfer(&mut d, &[0, 0, 0, 0]));
    d.set_input("temperature", 1372.0).unwrap();
    d.set_input("internal", -40.0).unwrap();
    t.extend(spi_xfer(&mut d, &[0, 0, 0, 0]));
    d.set_input("temperature", -270.0).unwrap();
    d.set_input("internal", 125.0).unwrap();
    t.extend(spi_xfer(&mut d, &[0, 0, 0, 0]));
    check("max31855", &t, MAX31855_GOLDEN);
}

const MAX31855_GOLDEN: &[u8] = &[
    0x01, 0x90, 0x16, 0x00, 0x55, 0xC0, 0xD8, 0x00, 0xEF, 0x20, 0x7D, 0x00,
];
