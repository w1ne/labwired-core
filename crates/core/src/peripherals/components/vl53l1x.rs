// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! VL53L1X laser time-of-flight distance sensor (I2C, default address 0x29).
//!
//! Behavioral model: responds to the register subset the Pololu VL53L1X Arduino
//! driver actually touches — the `init()` identity check, `SYSTEM__MODE_START`,
//! the `dataReady()` poll, and the 17-byte result burst — which is enough for
//! firmware built on that driver to initialise the sensor and read a distance.
//!
//! The ranging algorithm itself is NOT simulated; the reported distance is a
//! host-settable value (see [`Vl53l1x::set_distance_mm`]), mirroring the
//! static-sample-plus-host-stimulus approach of the MPU6050 model.
//!
//! Register addresses and the init/read code path were verified against the
//! upstream `pololu/vl53l1x-arduino` driver (`VL53L1X.h` / `VL53L1X.cpp`):
//!   * `init()` aborts unless `readReg16Bit(IDENTIFICATION__MODEL_ID) == 0xEACC`
//!   * `startContinuous()` writes `0x40` to `SYSTEM__MODE_START`
//!   * `dataReady()` returns `(readReg(GPIO__TIO_HV_STATUS) & 0x01) == 0`
//!   * `readResults()` burst-reads 17 bytes from `RESULT__RANGE_STATUS`; byte 0
//!     is the raw range status (`9` ⇒ `RangeValid` when stream_count != 0) and
//!     bytes 13..15 are the final range in mm, big-endian
//!   * `read()` back-converts the raw range as `(raw * 2011 + 0x400) / 0x800`

use crate::peripherals::i2c::I2cDevice;

// ── Register addresses (16-bit register pointer) ─────────────────────────────
const REG_GPIO_TIO_HV_STATUS: u16 = 0x0031;
const REG_SYSTEM_INTERRUPT_CLEAR: u16 = 0x0086;
const REG_SYSTEM_MODE_START: u16 = 0x0087;
const REG_RESULT_RANGE_STATUS: u16 = 0x0089; // base of the 17-byte result burst
const REG_IDENTIFICATION_MODEL_ID: u16 = 0x010F; // hi @0x010F, lo @0x0110

/// Identity value `init()` requires: `readReg16Bit(MODEL_ID) == 0xEACC`.
const MODEL_ID: u16 = 0xEACC;
/// Raw `RESULT__RANGE_STATUS` byte that the driver maps to `RangeValid`
/// (only when `stream_count` is non-zero).
const RANGE_STATUS_VALID: u8 = 9;

/// VL53L1X time-of-flight distance sensor model.
#[derive(Debug, serde::Serialize)]
pub struct Vl53l1x {
    address: u8,

    /// Latched 16-bit register pointer. Persists across the repeated-START that
    /// separates a pointer write from the following read burst.
    reg_ptr: u16,
    /// Number of address bytes collected in the current transaction. The
    /// VL53L1X uses a 2-byte register pointer, so the first two written bytes
    /// form the address and the rest are data. Reset on START/STOP.
    addr_bytes: u8,

    /// Distance the model reports, in millimetres. Host-settable stimulus
    /// (mirrors `Mpu6050::set_sample`); the ranging algorithm is not simulated.
    distance_mm: u16,
    /// True once firmware has written `SYSTEM__MODE_START` (ranging running).
    ranging: bool,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Vl53l1x {
    fn default() -> Self {
        Self::new(0x29) // VL53L1X fixed I2C address
    }
}

impl Vl53l1x {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            reg_ptr: 0,
            addr_bytes: 0,
            distance_mm: 500,
            ranging: false,
            component_id: None,
        }
    }

    /// Set the distance the sensor reports, in millimetres. Host stimulus hook;
    /// the WASM/host bridge drives this for live updates the same way the
    /// MPU6050 model takes injected samples.
    pub fn set_distance_mm(&mut self, mm: u16) {
        self.distance_mm = mm;
    }

    pub fn distance_mm(&self) -> u16 {
        self.distance_mm
    }

    /// Raw range register value the driver back-converts to `distance_mm`.
    ///
    /// The driver computes `range_mm = (raw * 2011 + 0x400) / 0x800`, so to make
    /// it report `distance_mm` we invert: `raw = round((mm * 0x800 - 0x400) / 2011)`.
    fn raw_range(&self) -> u16 {
        let mm = self.distance_mm as u32;
        let num = mm.saturating_mul(0x0800).saturating_sub(0x0400);
        // Rounded integer division.
        ((num + 2011 / 2) / 2011) as u16
    }

    fn read_register(&self, reg: u16) -> u8 {
        match reg {
            // 16-bit MODEL_ID, read as two bytes (hi then lo) → 0xEACC.
            REG_IDENTIFICATION_MODEL_ID => (MODEL_ID >> 8) as u8,
            0x0110 => (MODEL_ID & 0xFF) as u8,

            // dataReady() == (reg & 0x01) == 0. Report ready (even) once ranging
            // has been started, not-ready (odd) before that.
            REG_GPIO_TIO_HV_STATUS => {
                if self.ranging {
                    0x00
                } else {
                    0x01
                }
            }

            // 17-byte result burst from RESULT__RANGE_STATUS (0x0089):
            //   off 0  (0x0089): raw range status (9 = valid)
            //   off 2  (0x008B): stream_count (must be != 0 for RangeValid)
            //   off 13 (0x0096): final range mm, high byte
            //   off 14 (0x0097): final range mm, low byte
            REG_RESULT_RANGE_STATUS => RANGE_STATUS_VALID,
            0x008B => 1, // stream_count, non-zero
            0x0096 => (self.raw_range() >> 8) as u8,
            0x0097 => (self.raw_range() & 0xFF) as u8,

            // Everything else in the result block (and unmodeled registers) reads 0.
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u16, value: u8) {
        match reg {
            REG_SYSTEM_MODE_START => self.ranging = value != 0,
            REG_SYSTEM_INTERRUPT_CLEAR => { /* IRQ ack — no modeled state */ }
            // The driver's init() writes ~135 config registers; accept and
            // ignore them. The ranging algorithm is not simulated.
            _ => {}
        }
    }
}

impl I2cDevice for Vl53l1x {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let val = self.read_register(self.reg_ptr);
        self.reg_ptr = self.reg_ptr.wrapping_add(1);
        val
    }

    fn write(&mut self, data: u8) {
        match self.addr_bytes {
            0 => {
                self.reg_ptr = (data as u16) << 8;
                self.addr_bytes = 1;
            }
            1 => {
                self.reg_ptr = (self.reg_ptr & 0xFF00) | data as u16;
                self.addr_bytes = 2;
            }
            _ => {
                self.write_register(self.reg_ptr, data);
                self.reg_ptr = self.reg_ptr.wrapping_add(1);
            }
        }
    }

    fn start(&mut self) {
        // New transaction: collect a fresh 2-byte register pointer. The latched
        // reg_ptr is preserved so a repeated-START read continues from it.
        self.addr_bytes = 0;
    }

    fn stop(&mut self) {
        self.addr_bytes = 0;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Drivable target distance, in mm (VL53L1X long-range ceiling ~4 m). One
/// table backs BOTH the `SimInput` impl and the kit metadata, so the device
/// schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "distance",
    label: "Distance",
    unit: "mm",
    min: 0.0,
    max: 4000.0,
}];

impl crate::sim_input::SimInput for Vl53l1x {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.set_distance_mm(value.round() as u16);
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Vl53l1xKit;
pub static VL53L1X_KIT: Vl53l1xKit = Vl53l1xKit;

static VL53L1X_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "vl53l1x",
    label: "VL53L1X ToF",
    summary: "Laser time-of-flight distance sensor over I2C.",
    detail: "STMicroelectronics VL53L1X at fixed address 0x29, modeled against the \
             Pololu VL53L1X driver: MODEL_ID = 0xEACC, SYSTEM__MODE_START to begin \
             ranging, GPIO__TIO_HV_STATUS data-ready poll, and the RESULT range burst. \
             Reports a host-settable distance (default 500 mm); the ranging algorithm \
             is not simulated.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to the VL53L1X fixed address 0x29.",
    }],
    labs: &[LabRef {
        board_id: "vl53l1x-tof-lab",
        chip: "stm32f103",
        example_dir: "vl53l1x-tof-lab",
        demo_elf: "demo-vl53l1x-tof-lab.elf",
    }],
};

impl PeripheralKit for Vl53l1xKit {
    fn metadata(&self) -> &'static KitMetadata {
        &VL53L1X_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x29)?;
        ctx.attach_i2c_device(Box::new(Vl53l1x::new(address)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a 16-bit-pointer register write transaction: START, 2 address
    /// bytes, optional data bytes, STOP.
    fn write_reg(dev: &mut Vl53l1x, reg: u16, data: &[u8]) {
        dev.start();
        dev.write((reg >> 8) as u8);
        dev.write((reg & 0xFF) as u8);
        for &b in data {
            dev.write(b);
        }
        dev.stop();
    }

    /// Drive a pointer-set + burst read: write the 16-bit pointer (STOP), then a
    /// repeated-START read of `n` bytes.
    fn read_burst(dev: &mut Vl53l1x, reg: u16, n: usize) -> Vec<u8> {
        write_reg(dev, reg, &[]);
        dev.start();
        let out = (0..n).map(|_| dev.read()).collect();
        dev.stop();
        out
    }

    #[test]
    fn default_address_is_0x29() {
        assert_eq!(Vl53l1x::default().address(), 0x29);
    }

    #[test]
    fn model_id_reads_0xeacc() {
        // init() does readReg16Bit(0x010F) and aborts unless it equals 0xEACC.
        let mut dev = Vl53l1x::default();
        let bytes = read_burst(&mut dev, REG_IDENTIFICATION_MODEL_ID, 2);
        let id = ((bytes[0] as u16) << 8) | bytes[1] as u16;
        assert_eq!(id, 0xEACC);
    }

    #[test]
    fn data_ready_gated_on_mode_start() {
        let mut dev = Vl53l1x::default();
        // dataReady() == (reg & 0x01) == 0. Before ranging: odd (not ready).
        assert_eq!(read_burst(&mut dev, REG_GPIO_TIO_HV_STATUS, 1)[0] & 0x01, 1);
        // startContinuous() writes 0x40 to SYSTEM__MODE_START.
        write_reg(&mut dev, REG_SYSTEM_MODE_START, &[0x40]);
        assert_eq!(read_burst(&mut dev, REG_GPIO_TIO_HV_STATUS, 1)[0] & 0x01, 0);
    }

    #[test]
    fn result_burst_reports_injected_distance() {
        let mut dev = Vl53l1x::default();
        dev.set_distance_mm(742);
        write_reg(&mut dev, REG_SYSTEM_MODE_START, &[0x40]);

        let r = read_burst(&mut dev, REG_RESULT_RANGE_STATUS, 17);
        assert_eq!(r.len(), 17);
        // byte 0: raw range status maps to RangeValid (9), with stream_count != 0.
        assert_eq!(r[0], RANGE_STATUS_VALID);
        assert_ne!(r[2], 0, "stream_count must be non-zero for RangeValid");

        // bytes 13..15: final range mm (big-endian raw), back-converted exactly
        // as the Pololu driver does in read().
        let raw = ((r[13] as u32) << 8) | r[14] as u32;
        let range_mm = (raw * 2011 + 0x0400) / 0x0800;
        // Allow ±1 mm for the integer round-trip through the 2011/2048 scaling.
        assert!(
            (range_mm as i32 - 742).abs() <= 1,
            "decoded {range_mm} mm, expected 742"
        );
    }

    #[test]
    fn config_writes_are_accepted_and_ignored() {
        // The driver's init() blasts ~135 config registers; the model must
        // accept arbitrary writes without panicking or corrupting the pointer.
        let mut dev = Vl53l1x::default();
        write_reg(
            &mut dev,
            0x002D,
            &[0x00, 0x01, 0x01, 0x01, 0x02, 0x00, 0x02, 0x08],
        );
        dev.set_distance_mm(300);
        write_reg(&mut dev, REG_SYSTEM_MODE_START, &[0x40]);
        let r = read_burst(&mut dev, REG_RESULT_RANGE_STATUS, 17);
        let raw = ((r[13] as u32) << 8) | r[14] as u32;
        let range_mm = (raw * 2011 + 0x0400) / 0x0800;
        assert!((range_mm as i32 - 300).abs() <= 1);
    }
}
