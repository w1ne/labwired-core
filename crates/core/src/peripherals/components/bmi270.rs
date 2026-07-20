// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Register-level Bosch BMI270 6-axis IMU (3-axis accel + 3-axis gyro).
//!
//! Every register offset and reset value here is taken from the Bosch BMI270
//! datasheet (rev 1.x) — NOT from BMI160/BMA, which differ. The model is a
//! behavioural I²C slave: a register pointer, an auto-incrementing read burst,
//! and a `SimInput` surface so tests/hosts can drive accel/gyro/temperature and
//! the step count in engineering units.
//!
//! The defining BMI270 quirk is its **config-load handshake**: out of reset the
//! feature engine is un-initialised (`INTERNAL_STATUS.message == not_init`); the
//! host must softreset, disable advanced power save, then burst-upload the ~8 KB
//! config image to `INIT_DATA` and finally write `INIT_CTRL = 1`, after which
//! `INTERNAL_STATUS` reads `init_ok` (0x01). A firmware that skips the upload
//! must NOT see `init_ok` — that anti-circular gate is modelled faithfully. The
//! config-blob contents are accepted and discarded (real init just streams them
//! into internal memory).

use crate::peripherals::i2c::I2cDevice;

// ─── I²C address ────────────────────────────────────────────────────────────

/// Default 7-bit slave address (SDO tied low). 0x69 selects the SDO-high variant.
pub const BMI270_ADDR: u8 = 0x68;
/// `CHIP_ID` (0x00) value that identifies a BMI270.
pub const BMI270_CHIP_ID: u8 = 0x24;

// ─── Register map (datasheet) ───────────────────────────────────────────────

const REG_CHIP_ID: u8 = 0x00;
const REG_ERR: u8 = 0x02;
const REG_STATUS: u8 = 0x03;
// ACC data 0x0C..0x11, GYR data 0x12..0x17, SENSORTIME 0x18..0x1A — handled by range.
const REG_INTERNAL_STATUS: u8 = 0x21;
// TEMPERATURE 0x22..0x23 — handled inline.
const REG_FEAT_PAGE: u8 = 0x2F;
const REG_ACC_CONF: u8 = 0x40;
const REG_ACC_RANGE: u8 = 0x41;
const REG_GYR_CONF: u8 = 0x42;
const REG_GYR_RANGE: u8 = 0x43;
const REG_INIT_CTRL: u8 = 0x59;
const REG_INIT_ADDR_0: u8 = 0x5B;
const REG_INIT_ADDR_1: u8 = 0x5C;
const REG_INIT_DATA: u8 = 0x5E;
const REG_PWR_CONF: u8 = 0x7C;
const REG_PWR_CTRL: u8 = 0x7D;
const REG_CMD: u8 = 0x7E;

// ─── STATUS (0x03) bits ─────────────────────────────────────────────────────

const STATUS_DRDY_ACC: u8 = 0x80; // bit7
const STATUS_DRDY_GYR: u8 = 0x40; // bit6
const STATUS_CMD_RDY: u8 = 0x10; // bit4

// ─── PWR_CTRL (0x7D) bits ───────────────────────────────────────────────────

const PWR_CTRL_GYR_EN: u8 = 0x02; // bit1
const PWR_CTRL_ACC_EN: u8 = 0x04; // bit2

// ─── INTERNAL_STATUS (0x21) message field [3:0] ─────────────────────────────

const INTERNAL_STATUS_NOT_INIT: u8 = 0x00;
const INTERNAL_STATUS_INIT_OK: u8 = 0x01;

// ─── INIT_CTRL (0x59) ───────────────────────────────────────────────────────

const INIT_CTRL_LOAD_DONE: u8 = 0x01; // written after the config blob is streamed

// ─── CMD (0x7E) ─────────────────────────────────────────────────────────────

const CMD_FIFO_FLUSH: u8 = 0xB0;
const CMD_SOFTRESET: u8 = 0xB6;

// ─── POR reset values (datasheet) ───────────────────────────────────────────

const PWR_CONF_RESET: u8 = 0x03; // adv_power_save=1, fifo_self_wakeup=1
const ACC_CONF_RESET: u8 = 0xA8;
const ACC_RANGE_RESET: u8 = 0x02; // ±8 g (BMI270 default)
const GYR_CONF_RESET: u8 = 0xA9;
const GYR_RANGE_RESET: u8 = 0x00; // ±2000 dps (BMI270 default)

// ─── Feature-engine paging (step counter) ───────────────────────────────────

// DATASHEET: the feature-engine outputs (step counter, activity, wrist gesture…)
// are reached through a paged window: FEAT_PAGE (0x2F) selects one of 8 pages and
// the 16-byte FEATURES window (0x30..0x3F) exposes that page's registers. The
// exact page and in-page offset of a given output are fixed by the loaded config
// image (in the Bosch BMI2 sensor API the step-counter output is `start_addr` 0x00
// in the output map). We model the 32-bit little-endian step count at the base of
// the FEATURES window (0x30..0x33) on FEAT_PAGE = 6 — a stable, documented
// readout so firmware can obtain the step count deterministically.
const FEAT_PAGE_STEP: u8 = 6;

// ─── Sensitivity / encoding ─────────────────────────────────────────────────

// 16-bit signed data spans ±full-scale, i.e. 32768 LSB across the positive
// range: LSB/g = 32768 / fs_g, LSB/dps = 32768 / fs_dps.
const FULL_SCALE_LSB: f64 = 32768.0;
// TEMPERATURE: int16 LE, 0x0000 == 23 °C, 512 LSB/°C.
const TEMP_ZERO_C: f64 = 23.0;
const TEMP_LSB_PER_C: f64 = 512.0;

/// Register-level Bosch BMI270 6-axis IMU.
#[derive(Debug, serde::Serialize)]
pub struct Bmi270 {
    address: u8,

    // I²C register-pointer state machine.
    reg_ptr: u8,
    reg_written: bool,

    // Feature-engine config-load handshake.
    internal_status: u8,
    init_data_uploaded: bool,
    init_ctrl: u8,
    init_addr_0: u8,
    init_addr_1: u8,

    // Power / config registers (readback-faithful).
    pwr_conf: u8,
    pwr_ctrl: u8,
    acc_conf: u8,
    acc_range: u8,
    gyr_conf: u8,
    gyr_range: u8,

    // Feature paging.
    feat_page: u8,

    // Sensor data (raw register form; driven via `SimInput`).
    accel_x: i16,
    accel_y: i16,
    accel_z: i16,
    gyro_x: i16,
    gyro_y: i16,
    gyro_z: i16,
    temperature: i16,
    steps: u32,
    sensortime: u32,

    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Bmi270 {
    fn default() -> Self {
        Self::new(BMI270_ADDR)
    }
}

impl Bmi270 {
    pub fn new(address: u8) -> Self {
        let mut dev = Self {
            address,
            reg_ptr: 0,
            reg_written: false,
            internal_status: INTERNAL_STATUS_NOT_INIT,
            init_data_uploaded: false,
            init_ctrl: 0,
            init_addr_0: 0,
            init_addr_1: 0,
            pwr_conf: 0,
            pwr_ctrl: 0,
            acc_conf: 0,
            acc_range: 0,
            gyr_conf: 0,
            gyr_range: 0,
            feat_page: 0,
            accel_x: 0,
            accel_y: 0,
            accel_z: 0,
            gyro_x: 0,
            gyro_y: 0,
            gyro_z: 0,
            temperature: 0,
            steps: 0,
            sensortime: 0,
            component_id: None,
        };
        dev.por_reset();
        dev
    }

    /// Restore the power-on-reset (POR) state — the effect of `CMD = softreset`.
    /// Leaves the I²C transaction latch alone (a STOP clears that separately) and
    /// keeps the fixed identity fields (`address`, `component_id`).
    fn por_reset(&mut self) {
        self.internal_status = INTERNAL_STATUS_NOT_INIT;
        self.init_data_uploaded = false;
        self.init_ctrl = 0x00;
        self.init_addr_0 = 0x00;
        self.init_addr_1 = 0x00;
        self.feat_page = 0x00;

        self.pwr_conf = PWR_CONF_RESET;
        self.pwr_ctrl = 0x00;
        self.acc_conf = ACC_CONF_RESET;
        self.acc_range = ACC_RANGE_RESET;
        self.gyr_conf = GYR_CONF_RESET;
        self.gyr_range = GYR_RANGE_RESET;

        self.accel_x = 0;
        self.accel_y = 0;
        self.accel_z = 0;
        self.gyro_x = 0;
        self.gyro_y = 0;
        self.gyro_z = 0;
        self.temperature = 0; // 0x0000 == 23 °C
        self.steps = 0;
        self.sensortime = 0;
    }

    /// STATUS (0x03): command interface always ready; data-ready bits track the
    /// per-sensor enables in PWR_CTRL (fresh data is produced while enabled).
    fn status_byte(&self) -> u8 {
        let mut s = STATUS_CMD_RDY;
        if self.pwr_ctrl & PWR_CTRL_ACC_EN != 0 {
            s |= STATUS_DRDY_ACC;
        }
        if self.pwr_ctrl & PWR_CTRL_GYR_EN != 0 {
            s |= STATUS_DRDY_GYR;
        }
        s
    }

    /// FEATURES window (0x30..0x3F): 32-bit LE step count on the step page.
    fn read_features(&self, reg: u8) -> u8 {
        if self.feat_page == FEAT_PAGE_STEP {
            match reg {
                0x30 => (self.steps & 0xFF) as u8,
                0x31 => ((self.steps >> 8) & 0xFF) as u8,
                0x32 => ((self.steps >> 16) & 0xFF) as u8,
                0x33 => ((self.steps >> 24) & 0xFF) as u8,
                _ => 0,
            }
        } else {
            0
        }
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            REG_CHIP_ID => BMI270_CHIP_ID,
            REG_ERR => 0x00,
            REG_STATUS => self.status_byte(),

            0x0C => (self.accel_x & 0xFF) as u8,
            0x0D => (self.accel_x >> 8) as u8,
            0x0E => (self.accel_y & 0xFF) as u8,
            0x0F => (self.accel_y >> 8) as u8,
            0x10 => (self.accel_z & 0xFF) as u8,
            0x11 => (self.accel_z >> 8) as u8,

            0x12 => (self.gyro_x & 0xFF) as u8,
            0x13 => (self.gyro_x >> 8) as u8,
            0x14 => (self.gyro_y & 0xFF) as u8,
            0x15 => (self.gyro_y >> 8) as u8,
            0x16 => (self.gyro_z & 0xFF) as u8,
            0x17 => (self.gyro_z >> 8) as u8,

            0x18 => (self.sensortime & 0xFF) as u8,
            0x19 => ((self.sensortime >> 8) & 0xFF) as u8,
            0x1A => ((self.sensortime >> 16) & 0xFF) as u8,

            REG_INTERNAL_STATUS => self.internal_status,

            0x22 => (self.temperature & 0xFF) as u8,
            0x23 => (self.temperature >> 8) as u8,

            REG_FEAT_PAGE => self.feat_page,
            0x30..=0x3F => self.read_features(reg),

            REG_ACC_CONF => self.acc_conf,
            REG_ACC_RANGE => self.acc_range,
            REG_GYR_CONF => self.gyr_conf,
            REG_GYR_RANGE => self.gyr_range,

            REG_INIT_CTRL => self.init_ctrl,
            REG_INIT_ADDR_0 => self.init_addr_0,
            REG_INIT_ADDR_1 => self.init_addr_1,

            REG_PWR_CONF => self.pwr_conf,
            REG_PWR_CTRL => self.pwr_ctrl,

            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            REG_FEAT_PAGE => self.feat_page = value,
            REG_ACC_CONF => self.acc_conf = value,
            REG_ACC_RANGE => self.acc_range = value & 0x03,
            REG_GYR_CONF => self.gyr_conf = value,
            REG_GYR_RANGE => self.gyr_range = value & 0x07,

            REG_INIT_CTRL => {
                self.init_ctrl = value;
                // The config-load gate: init_ok only when the blob was actually
                // streamed. A firmware that skips the upload sees not_init.
                if value == INIT_CTRL_LOAD_DONE {
                    self.internal_status = if self.init_data_uploaded {
                        INTERNAL_STATUS_INIT_OK
                    } else {
                        INTERNAL_STATUS_NOT_INIT
                    };
                }
            }
            REG_INIT_ADDR_0 => self.init_addr_0 = value,
            REG_INIT_ADDR_1 => self.init_addr_1 = value,
            REG_INIT_DATA => self.init_data_uploaded = true, // accept + discard blob

            REG_PWR_CONF => self.pwr_conf = value,
            REG_PWR_CTRL => self.pwr_ctrl = value,

            REG_CMD => self.exec_cmd(value),

            // Read-only (CHIP_ID, data, STATUS, INTERNAL_STATUS, TEMPERATURE …)
            // and unmapped registers ignore writes.
            _ => {}
        }
    }

    fn exec_cmd(&mut self, cmd: u8) {
        match cmd {
            CMD_SOFTRESET => self.por_reset(),
            CMD_FIFO_FLUSH => {} // no FIFO modelled
            _ => {}
        }
    }

    /// Convert g → raw LSB using the LIVE `ACC_RANGE` (±2/4/8/16 g), clamped to
    /// full-scale like the silicon.
    fn accel_to_lsb(&self, g: f64) -> i16 {
        let code = (self.acc_range & 0x03) as u32;
        let fs_g = (2u32 << code) as f64; // 2, 4, 8, 16
        let lsb_per_g = FULL_SCALE_LSB / fs_g;
        (g.clamp(-fs_g, fs_g) * lsb_per_g).round() as i16
    }

    /// Convert dps → raw LSB using the LIVE `GYR_RANGE` (±2000/1000/500/250/125),
    /// clamped to full-scale like the silicon.
    fn gyro_to_lsb(&self, dps: f64) -> i16 {
        let code = (self.gyr_range & 0x07) as u32;
        let fs_dps = (2000u32 >> code) as f64; // 2000, 1000, 500, 250, 125
        let lsb_per_dps = FULL_SCALE_LSB / fs_dps;
        (dps.clamp(-fs_dps, fs_dps) * lsb_per_dps).round() as i16
    }
}

impl I2cDevice for Bmi270 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let val = self.read_register(self.reg_ptr);
        self.reg_ptr = self.reg_ptr.wrapping_add(1);
        val
    }

    fn write(&mut self, data: u8) {
        if !self.reg_written {
            // First byte after START is the register pointer.
            self.reg_ptr = data;
            self.reg_written = true;
        } else {
            self.write_register(self.reg_ptr, data);
            // INIT_DATA is a streaming port: the config image advances an
            // internal pointer (INIT_ADDR), so the I²C register pointer stays put
            // for the whole ~8 KB burst. Every other register auto-increments.
            if self.reg_ptr != REG_INIT_DATA {
                self.reg_ptr = self.reg_ptr.wrapping_add(1);
            }
        }
    }

    fn stop(&mut self) {
        self.reg_written = false;
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

/// Drivable channels. Accel/gyro follow the LIVE full-scale the firmware
/// configured; values beyond it saturate, like the silicon. One table backs
/// both the `SimInput` impl and any kit metadata, so schema and runtime cannot
/// drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "ax",
        label: "Accel X",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "ay",
        label: "Accel Y",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "az",
        label: "Accel Z",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "gx",
        label: "Gyro X",
        unit: "°/s",
        min: -2000.0,
        max: 2000.0,
    },
    crate::sim_input::InputChannel {
        key: "gy",
        label: "Gyro Y",
        unit: "°/s",
        min: -2000.0,
        max: 2000.0,
    },
    crate::sim_input::InputChannel {
        key: "gz",
        label: "Gyro Z",
        unit: "°/s",
        min: -2000.0,
        max: 2000.0,
    },
    crate::sim_input::InputChannel {
        key: "temp",
        label: "Temperature",
        unit: "°C",
        // int16 register spans roughly [-41, +87] °C around the 23 °C zero.
        min: -41.0,
        max: 87.0,
    },
    crate::sim_input::InputChannel {
        key: "steps",
        label: "Step count",
        unit: "steps",
        min: 0.0,
        max: 4_294_967_295.0, // 32-bit step counter
    },
];

impl crate::sim_input::SimInput for Bmi270 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "ax" => self.accel_x = self.accel_to_lsb(value),
            "ay" => self.accel_y = self.accel_to_lsb(value),
            "az" => self.accel_z = self.accel_to_lsb(value),
            "gx" => self.gyro_x = self.gyro_to_lsb(value),
            "gy" => self.gyro_y = self.gyro_to_lsb(value),
            "gz" => self.gyro_z = self.gyro_to_lsb(value),
            "temp" => {
                self.temperature = ((value - TEMP_ZERO_C) * TEMP_LSB_PER_C).round() as i16;
            }
            "steps" => self.steps = value.max(0.0) as u32,
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    // ── I²C transaction helpers ─────────────────────────────────────────────

    /// Point the register pointer at `reg` (a write transaction ending in STOP),
    /// then read one byte from it.
    fn read_reg(dev: &mut Bmi270, reg: u8) -> u8 {
        dev.write(reg);
        dev.stop();
        dev.read()
    }

    /// Read `n` bytes starting at `reg` in one burst (auto-increment).
    fn read_burst(dev: &mut Bmi270, reg: u8, n: usize) -> Vec<u8> {
        dev.write(reg);
        dev.stop();
        (0..n).map(|_| dev.read()).collect()
    }

    /// Write `val` to `reg` (a two-byte write transaction).
    fn write_reg(dev: &mut Bmi270, reg: u8, val: u8) {
        dev.write(reg);
        dev.write(val);
        dev.stop();
    }

    fn i16_le(lo: u8, hi: u8) -> i16 {
        i16::from_le_bytes([lo, hi])
    }

    #[test]
    fn chip_id_reads_0x24() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        assert_eq!(dev.address(), 0x68);
        assert_eq!(read_reg(&mut dev, REG_CHIP_ID), 0x24);
        assert_eq!(read_reg(&mut dev, REG_CHIP_ID), BMI270_CHIP_ID);
    }

    #[test]
    fn sdo_high_address_variant() {
        let dev = Bmi270::new(0x69);
        assert_eq!(dev.address(), 0x69);
    }

    #[test]
    fn softreset_restores_por_values() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        // Perturb a spread of registers.
        write_reg(&mut dev, REG_PWR_CTRL, 0x0E);
        write_reg(&mut dev, REG_ACC_RANGE, 0x00);
        write_reg(&mut dev, REG_GYR_RANGE, 0x03);
        write_reg(&mut dev, REG_PWR_CONF, 0x00);
        dev.set_input("steps", 1234.0).unwrap();
        dev.set_input("az", 1.0).unwrap();
        assert_ne!(read_reg(&mut dev, REG_PWR_CTRL), 0x00);

        // Softreset via CMD.
        write_reg(&mut dev, REG_CMD, CMD_SOFTRESET);

        assert_eq!(read_reg(&mut dev, REG_PWR_CTRL), 0x00);
        assert_eq!(read_reg(&mut dev, REG_PWR_CONF), PWR_CONF_RESET);
        assert_eq!(read_reg(&mut dev, REG_ACC_CONF), ACC_CONF_RESET);
        assert_eq!(read_reg(&mut dev, REG_ACC_RANGE), ACC_RANGE_RESET);
        assert_eq!(read_reg(&mut dev, REG_GYR_CONF), GYR_CONF_RESET);
        assert_eq!(read_reg(&mut dev, REG_GYR_RANGE), GYR_RANGE_RESET);
        assert_eq!(read_reg(&mut dev, REG_INTERNAL_STATUS), 0x00);
        // Sensor data cleared.
        assert_eq!(read_reg(&mut dev, 0x10), 0x00);
        assert_eq!(read_reg(&mut dev, 0x11), 0x00);
        // Step count cleared (read via the feature page).
        write_reg(&mut dev, REG_FEAT_PAGE, FEAT_PAGE_STEP);
        assert_eq!(read_burst(&mut dev, 0x30, 4), vec![0, 0, 0, 0]);
    }

    #[test]
    fn config_load_handshake_gate() {
        let mut dev = Bmi270::new(BMI270_ADDR);

        // 1. Softreset → POR: not_init.
        write_reg(&mut dev, REG_CMD, CMD_SOFTRESET);
        assert_eq!(read_reg(&mut dev, REG_INTERNAL_STATUS), 0x00);

        // 2/3. Disable adv power save, arm the loader.
        write_reg(&mut dev, REG_PWR_CONF, 0x00);
        write_reg(&mut dev, REG_INIT_CTRL, 0x00);
        // Still not_init before the upload + load-done.
        assert_eq!(read_reg(&mut dev, REG_INTERNAL_STATUS), 0x00);

        // 4. Burst-upload the config blob to INIT_DATA. Include a 0xB6 byte to
        //    prove the register pointer does NOT wander into CMD/softreset.
        dev.write(REG_INIT_DATA);
        for b in [0x00u8, 0xB6, 0x2E, 0x00, 0xFF, 0xB6] {
            dev.write(b);
        }
        dev.stop();
        // Upload alone (before INIT_CTRL=1) does not flip the gate.
        assert_eq!(read_reg(&mut dev, REG_INTERNAL_STATUS), 0x00);

        // 5. Load done.
        write_reg(&mut dev, REG_INIT_CTRL, 0x01);

        // 6. init_ok.
        assert_eq!(read_reg(&mut dev, REG_INTERNAL_STATUS), 0x01);
    }

    #[test]
    fn skipping_upload_never_reaches_init_ok() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        write_reg(&mut dev, REG_CMD, CMD_SOFTRESET);
        // Firmware writes INIT_CTRL=1 WITHOUT streaming the blob.
        write_reg(&mut dev, REG_INIT_CTRL, 0x01);
        assert_eq!(read_reg(&mut dev, REG_INTERNAL_STATUS), 0x00);
    }

    #[test]
    fn accel_readback_matches_siminput() {
        let mut dev = Bmi270::new(BMI270_ADDR); // ACC_RANGE POR = ±8 g → 4096 LSB/g
        dev.set_input("ax", 1.0).unwrap();
        dev.set_input("ay", -1.0).unwrap();
        dev.set_input("az", 2.0).unwrap();

        let d = read_burst(&mut dev, 0x0C, 6);
        assert_eq!(i16_le(d[0], d[1]), 4096); // +1 g
        assert_eq!(i16_le(d[2], d[3]), -4096); // -1 g (sign preserved)
        assert_eq!(i16_le(d[4], d[5]), 8192); // +2 g
    }

    #[test]
    fn gyro_readback_matches_siminput() {
        let mut dev = Bmi270::new(BMI270_ADDR); // GYR_RANGE POR = ±2000 dps → 16.384 LSB/dps
        dev.set_input("gx", 250.0).unwrap();
        dev.set_input("gy", -250.0).unwrap();
        dev.set_input("gz", 0.0).unwrap();

        let d = read_burst(&mut dev, 0x12, 6);
        assert_eq!(i16_le(d[0], d[1]), (250.0 * 16.384_f64).round() as i16);
        assert_eq!(i16_le(d[2], d[3]), (-250.0 * 16.384_f64).round() as i16);
        assert_eq!(i16_le(d[4], d[5]), 0);
    }

    #[test]
    fn accel_scale_follows_configured_range() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        write_reg(&mut dev, REG_ACC_RANGE, 0x00); // ±2 g → 16384 LSB/g
        dev.set_input("ax", 1.0).unwrap();
        let d = read_burst(&mut dev, 0x0C, 2);
        assert_eq!(i16_le(d[0], d[1]), 16384);
    }

    #[test]
    fn temperature_encodes_celsius() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        // 23 °C is the zero point.
        dev.set_input("temp", 23.0).unwrap();
        let d = read_burst(&mut dev, 0x22, 2);
        assert_eq!(i16_le(d[0], d[1]), 0);

        // +25 °C: (25 - 23) * 512 = 1024.
        dev.set_input("temp", 25.0).unwrap();
        let d = read_burst(&mut dev, 0x22, 2);
        assert_eq!(i16_le(d[0], d[1]), 1024);

        // Below zero point stays signed.
        dev.set_input("temp", 21.0).unwrap();
        let d = read_burst(&mut dev, 0x22, 2);
        assert_eq!(i16_le(d[0], d[1]), -1024);
    }

    #[test]
    fn step_counter_readout_via_feature_page() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        dev.set_input("steps", 100_000.0).unwrap();

        // Wrong page reads zero.
        write_reg(&mut dev, REG_FEAT_PAGE, 0x00);
        assert_eq!(read_burst(&mut dev, 0x30, 4), vec![0, 0, 0, 0]);

        // Step page yields the 32-bit LE count.
        write_reg(&mut dev, REG_FEAT_PAGE, FEAT_PAGE_STEP);
        let d = read_burst(&mut dev, 0x30, 4);
        let steps = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
        assert_eq!(steps, 100_000);
    }

    #[test]
    fn reg_pointer_auto_increments_across_burst() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        dev.set_input("ax", 1.0).unwrap();
        dev.set_input("gz", 100.0).unwrap();

        // One 12-byte burst crossing accel (0x0C..0x11) into gyro (0x12..0x17).
        let d = read_burst(&mut dev, 0x0C, 12);
        assert_eq!(i16_le(d[0], d[1]), 4096); // accel_x
        assert_eq!(
            i16_le(d[10], d[11]),
            (100.0 * 16.384_f64).round() as i16 // gyro_z, proving the pointer walked
        );
    }

    #[test]
    fn writing_read_only_register_is_ignored() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        write_reg(&mut dev, REG_CHIP_ID, 0x00); // attempt to clobber CHIP_ID
        assert_eq!(read_reg(&mut dev, REG_CHIP_ID), 0x24);
    }

    #[test]
    fn status_data_ready_tracks_enables() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        // POR: acc/gyr disabled, only cmd_rdy set.
        assert_eq!(read_reg(&mut dev, REG_STATUS), STATUS_CMD_RDY);

        write_reg(&mut dev, REG_PWR_CTRL, PWR_CTRL_ACC_EN | PWR_CTRL_GYR_EN);
        let s = read_reg(&mut dev, REG_STATUS);
        assert_ne!(s & STATUS_DRDY_ACC, 0);
        assert_ne!(s & STATUS_DRDY_GYR, 0);
        assert_ne!(s & STATUS_CMD_RDY, 0);
    }

    #[test]
    fn siminput_rejects_unknown_and_out_of_range() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        assert!(dev.set_input("nope", 1.0).is_err());
        assert!(dev.set_input("ax", 99.0).is_err()); // beyond ±16 g channel max
        assert!(dev.set_input("ax", 4.0).is_ok());
    }

    #[test]
    fn component_id_round_trips() {
        let mut dev = Bmi270::new(BMI270_ADDR);
        assert_eq!(dev.component_id(), None);
        dev.set_component_id("imu".to_string());
        assert_eq!(dev.component_id(), Some("imu"));
    }
}
