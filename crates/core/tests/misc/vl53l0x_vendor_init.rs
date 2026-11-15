// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! VL53L0X: the register surface ST's OWN initialisation sequence walks.
//!
//! `Adafruit_VL53L0X::begin()` is a thin wrapper over the ST API 1.0.2 flow —
//! `VL53L0X_DataInit` → `GetDeviceInfo` → `StaticInit` →
//! `PerformRefSpadManagement` → `PerformRefCalibration` — and every stage of it
//! polls something. A descriptor that models only the datasheet one-shot range
//! answers `unmapped_byte` to all of it, and the library either times out or
//! bails with `VL53L0X_ERROR_REF_SPAD_INIT`.
//!
//! Each test here pins ONE thing the vendor flow cannot get past, and names the
//! ST function that would fail without it. They are unit-level on purpose: the
//! end-to-end proof is running the stock library on a real ESP32-C3 image, which
//! is far too slow for a test suite, so this file locks the wire behaviour that
//! run depends on.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

const ADDR: u8 = 0x29;

/// ST's `targetRefRate` (`VL53L0X_DataInit`): 20 MCPS in 9.7 fixed point.
const TARGET_REF_RATE: u16 = 0x0A00;
/// ST's `VL53L0X_REG_SYSTEM_INTERRUPT_GPIO_NEW_SAMPLE_READY`.
const NEW_SAMPLE_READY: u8 = 0x04;

fn device() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("vl53l0x")
        .expect("vl53l0x descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("vl53l0x.yaml does not build")
}

/// `VL53L0X_ReadMulti`: point at `reg`, pull `n` bytes.
fn read_block(dev: &mut GenericI2cDevice, reg: u8, n: usize) -> Vec<u8> {
    dev.stop();
    dev.start();
    dev.write(reg);
    dev.start();
    let out = (0..n).map(|_| dev.read()).collect();
    dev.stop();
    out
}

/// `VL53L0X_WrByte`.
fn write_reg(dev: &mut GenericI2cDevice, reg: u8, value: u8) {
    dev.stop();
    dev.start();
    dev.write(reg);
    dev.write(value);
    dev.stop();
}

/// `VL53L0X_WriteMulti`: pointer then a burst of data bytes.
fn write_block(dev: &mut GenericI2cDevice, reg: u8, bytes: &[u8]) {
    dev.stop();
    dev.start();
    dev.write(reg);
    for &b in bytes {
        dev.write(b);
    }
    dev.stop();
}

/// Select a register bank the way ST's API does.
fn select_bank(dev: &mut GenericI2cDevice, bank: u8) {
    write_reg(dev, 0xFF, bank);
}

/// `VL53L0X_device_read_strobe` + `RdDWord(0x90)`: the factory NVM fetch, with
/// the strobe polled exactly as the vendor code polls it. Returns the word and
/// how many polls it took (ST gives up after `VL53L0X_DEFAULT_MAX_LOOP` = 200).
fn nvm_read(dev: &mut GenericI2cDevice, index: u8) -> (u32, usize) {
    select_bank(dev, 0x07);
    write_reg(dev, 0x94, index);
    write_reg(dev, 0x83, 0x00); // arm
    let mut polls = 0;
    loop {
        polls += 1;
        if read_block(dev, 0x83, 1)[0] != 0x00 {
            break;
        }
        assert!(
            polls < 200,
            "strobe never came back — VL53L0X_ERROR_TIME_OUT"
        );
    }
    write_reg(dev, 0x83, 0x01);
    let d = read_block(dev, 0x90, 4);
    (u32::from_be_bytes([d[0], d[1], d[2], d[3]]), polls)
}

/// Establish an honest microsecond source — see the note in
/// `vl53l0x_migration_parity.rs`.
fn with_clock(dev: &mut GenericI2cDevice) {
    dev.advance_time_us(1);
}

#[test]
fn the_bank_select_makes_0xb6_two_different_registers() {
    // ST writes GLOBAL_CONFIG_REF_EN_START_SELECT = 0xB4 at pointer 0xB6 in bank
    // 0 (`VL53L0X_perform_ref_spad_management`) and reads
    // RESULT_PEAK_SIGNAL_RATE_REF from pointer 0xB6 in bank 1
    // (`perform_ref_signal_measurement`). One flat register would hand the
    // driver back its own configuration byte where silicon hands it a rate.
    let mut dev = device();
    select_bank(&mut dev, 0x00);
    write_reg(&mut dev, 0xB6, 0xB4);
    assert_eq!(
        read_block(&mut dev, 0xB6, 1),
        vec![0xB4],
        "bank 0 is config"
    );

    select_bank(&mut dev, 0x01);
    assert_eq!(
        read_block(&mut dev, 0xB6, 2),
        vec![0x00, 0x00],
        "bank 1 is the measured reference rate, and no SPADs are enabled yet"
    );

    select_bank(&mut dev, 0x00);
    assert_eq!(
        read_block(&mut dev, 0xB6, 1),
        vec![0xB4],
        "the bank-1 read must not have disturbed the bank-0 register"
    );
}

#[test]
fn the_magic_unlock_write_to_0x00_does_not_start_a_range() {
    // Every ST measurement start is preceded by
    // `WrByte(0xFF,0x01); WrByte(0x00,0x00); WrByte(0x91,stop); WrByte(0x00,0x01)`.
    // Pointer 0x00 in bank 1 is NOT SYSRANGE_START. Decoding it as one would
    // fire a range 33 ms before the driver asked for one, and the interrupt
    // would already be asserted when it started polling.
    let mut dev = device();
    with_clock(&mut dev);
    select_bank(&mut dev, 0x01);
    write_reg(&mut dev, 0x00, 0x01);
    select_bank(&mut dev, 0x00);

    dev.advance_time_us(33_000);
    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![0x00],
        "a bank-1 write must not have started a conversion"
    );
}

#[test]
fn sysrange_start_bit_zero_is_momentary() {
    // `VL53L0X_StartMeasurement` writes the bit and then re-reads the register
    // in a loop commented "Wait until start bit has been cleared", giving up
    // with VL53L0X_ERROR_TIME_OUT after 200 polls. Silicon clears it once the
    // measurement is under way.
    let mut dev = device();
    with_clock(&mut dev);
    write_reg(&mut dev, 0x00, 0x01);
    assert_eq!(
        read_block(&mut dev, 0x00, 1),
        vec![0x00],
        "the start bit must not read back set"
    );

    // ...and the conversion it kicked off is still running.
    dev.advance_time_us(33_000);
    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![NEW_SAMPLE_READY],
        "self-clearing the go bit must not cancel the measurement"
    );

    // `VL53L0X_perform_single_ref_calibration` writes 0x01|vhv_init_byte; only
    // bit 0 is momentary, the calibration selector bits stay put.
    write_reg(&mut dev, 0x0B, 0x01);
    write_reg(&mut dev, 0x00, 0x41);
    assert_eq!(read_block(&mut dev, 0x00, 1), vec![0x40]);
}

#[test]
fn the_interrupt_status_reports_the_configured_source() {
    // `VL53L0X_GetMeasurementDataReady` compares the masked status for EQUALITY
    // with the GPIO functionality StaticInit programmed (0x04 = new sample
    // ready), so anything else — including "all three bits set" — reads as
    // never ready and times the vendor poll out.
    let mut dev = device();
    with_clock(&mut dev);
    write_reg(&mut dev, 0x0A, NEW_SAMPLE_READY); // SYSTEM_INTERRUPT_CONFIG_GPIO
    write_reg(&mut dev, 0x00, 0x01);
    dev.advance_time_us(33_000);

    let status = read_block(&mut dev, 0x13, 1)[0];
    assert_eq!(status & 0x07, NEW_SAMPLE_READY, "vendor equality test");
    assert_ne!(status & 0x07, 0, "datasheet one-shot 'any bit set' test");
}

#[test]
fn the_nvm_port_answers_the_strobe_handshake() {
    // Without this port `VL53L0X_device_read_strobe` spins 200 times and returns
    // VL53L0X_ERROR_TIME_OUT, which fails BOTH `VL53L0X_GetDeviceInfo` and
    // `VL53L0X_StaticInit` — the library cannot initialise at all.
    let mut dev = device();
    with_clock(&mut dev);

    // Before anything is armed the strobe reads clear: the model raises it
    // because a fetch completed, not because the bit is wired high.
    select_bank(&mut dev, 0x07);
    assert_eq!(read_block(&mut dev, 0x83, 1), vec![0x00]);

    let (spad_record, polls) = nvm_read(&mut dev, 0x6B);
    assert_eq!(polls, 1, "an on-die fetch lands before the first poll");

    // `VL53L0X_get_info_from_device`: count = bits 14:8, type = bit 15.
    let count = (spad_record >> 8) & 0x7F;
    let is_aperture = (spad_record >> 15) & 0x01;
    assert_eq!(is_aperture, 0);
    assert_eq!(count, 10);
    // `VL53L0X_StaticInit` rejects the record as "NVM value invalid" outside
    // these bounds and falls back to a full re-characterisation.
    assert!(
        is_aperture <= 1 && (is_aperture == 1 && count <= 32 || is_aperture == 0 && count <= 12),
        "the factory record must be one the vendor API accepts"
    );

    // Good-SPAD map: 0x24 carries bytes 0..3, 0x25 bytes 4..5. A blank map makes
    // `enable_ref_spads` fail immediately with VL53L0X_ERROR_REF_SPAD_INIT.
    let (lo, _) = nvm_read(&mut dev, 0x24);
    let (hi, _) = nvm_read(&mut dev, 0x25);
    let map = [
        (lo >> 24) as u8,
        (lo >> 16) as u8,
        (lo >> 8) as u8,
        lo as u8,
        (hi >> 24) as u8,
        (hi >> 16) as u8,
    ];
    assert_eq!(map, [0xFF; 6], "all SPADs good");

    // An index the twin has no factory value for latches 0 — the answer an
    // unprogrammed cell gives — rather than a fabricated serial number.
    assert_eq!(
        nvm_read(&mut dev, 0x77).0,
        0,
        "product-id word is not invented"
    );
}

#[test]
fn the_reference_spad_map_round_trips_a_block_write() {
    // `set_ref_spad_map` block-WRITES six bytes at 0xB0 and `get_ref_spad_map`
    // reads them straight back; `enable_ref_spads` compares them byte for byte
    // and fails the whole init with VL53L0X_ERROR_REF_SPAD_INIT on a mismatch.
    // This is the case that needed auto-increment on the WRITE side.
    let mut dev = device();
    let map = [0xFF, 0x03, 0x00, 0x00, 0x00, 0x00];
    write_block(&mut dev, 0xB0, &map);
    assert_eq!(read_block(&mut dev, 0xB0, 6), map.to_vec());

    let map2 = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20];
    write_block(&mut dev, 0xB0, &map2);
    assert_eq!(read_block(&mut dev, 0xB0, 6), map2.to_vec());
}

#[test]
fn the_reference_rate_scales_with_the_enabled_spads() {
    // The relation `VL53L0X_perform_ref_spad_management` closes its loop on: it
    // enables one more good SPAD at a time, re-measures
    // RESULT_PEAK_SIGNAL_RATE_REF, and stops when the rate reaches
    // targetRefRate. A constant either stalls that loop forever or pushes it
    // down the "signal too high even at the minimum" branch.
    let mut dev = device();
    let rate = |dev: &mut GenericI2cDevice| {
        select_bank(dev, 0x01);
        let b = read_block(dev, 0xB6, 2);
        let v = (u16::from(b[0]) << 8) | u16::from(b[1]);
        select_bank(dev, 0x00);
        v
    };

    // ST starts from `minimumSpadCount` = 3.
    write_block(&mut dev, 0xB0, &[0x07, 0, 0, 0, 0, 0]);
    assert_eq!(rate(&mut dev), 3 * 0x0100);
    assert!(
        rate(&mut dev) < TARGET_REF_RATE,
        "the loop must keep adding"
    );

    // The factory record says 10 reference SPADs; the model is calibrated so
    // that is exactly where ST's target is met — the part is self-consistent.
    write_block(&mut dev, 0xB0, &[0xFF, 0x03, 0, 0, 0, 0]);
    assert_eq!(rate(&mut dev), TARGET_REF_RATE);

    // One more SPAD passes the target, which is what ends the loop.
    write_block(&mut dev, 0xB0, &[0xFF, 0x07, 0, 0, 0, 0]);
    assert!(rate(&mut dev) > TARGET_REF_RATE);
}

#[test]
fn the_range_still_comes_from_the_distance_channel() {
    // Nothing the vendor-init surface added may disturb the contract this part
    // exists for: raw millimetres, 16-bit big-endian, at 0x1E, sourced from
    // SimInput — and reading it still does NOT acknowledge the interrupt.
    use labwired_core::sim_input::SimInput;
    let mut dev = device();
    with_clock(&mut dev);
    dev.set_input("distance", 777.0).expect("distance channel");

    write_reg(&mut dev, 0x00, 0x01);
    dev.advance_time_us(33_000);
    assert_eq!(read_block(&mut dev, 0x1E, 2), vec![0x03, 0x09]); // 777 = 0x0309
    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![NEW_SAMPLE_READY],
        "reading the range must NOT clear the interrupt"
    );
}
