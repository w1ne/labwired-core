// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP **FXOS8700CQ** 6-axis (3D accelerometer + 3D magnetometer) I2C sensor.
//!
//! This is the sensor soldered onto the FRDM-KW41Z board (I2C1, address 0x1f),
//! and it is the part a livestock ear-tag uses to track *activity* — motion,
//! rumination, eating — alongside body temperature. The model answers the exact
//! register sequence the unmodified upstream Zephyr `fxos8700` driver issues:
//! the WHOAMI probe (0xC7), the standby→config→active bring-up, and the burst
//! read of OUT_X/Y/Z. Accelerometer data is a deterministic "grazing cow"
//! pattern — Z held near +1 g (standing), X/Y gently oscillating — so the boot
//! log shows believable, changing activity rather than a frozen sample.
//!
//! Register map (subset the driver touches), from the FXOS8700 datasheet:
//!   STATUS 0x00 · OUT_X_MSB 0x01..OUT_Z_LSB 0x06 (14-bit, left-justified)
//!   WHO_AM_I 0x0D (=0xC7) · XYZ_DATA_CFG 0x0E · CTRL_REG1 0x2A · CTRL_REG2 0x2B
//!   M_OUT_X_MSB 0x33.. (mag) · TEMP 0x51 · M_CTRL_REG1 0x5B · M_CTRL_REG2 0x5C

use crate::peripherals::i2c::I2cDevice;

const REG_STATUS: u8 = 0x00;
const REG_OUT_X_MSB: u8 = 0x01;
const REG_WHO_AM_I: u8 = 0x0D;
const REG_M_OUT_X_MSB: u8 = 0x33;
const REG_TEMP: u8 = 0x51;
const WHOAMI_FXOS8700: u8 = 0xC7;

/// +1 g on the Z axis, as the raw 16-bit (14-bit left-justified) register pair.
/// The Zephyr fxos8700 sample reports the default ±8 g range (1024 counts/g);
/// 1024 counts << 2 = 4096 = 0x1000 reads back as ≈ 9.8 m/s², i.e. a tag lying
/// flat / a standing animal.
const ONE_G_LJ: i16 = 0x1000;

#[derive(Debug, serde::Serialize)]
pub struct Fxos8700 {
    address: u8,
    current_register: u8,
    register_address_written: bool,

    /// Config registers the driver writes during bring-up (kept so reads of
    /// them — e.g. reg_field_update's read-modify-write — return what was set).
    ctrl_reg1: u8,
    ctrl_reg2: u8,
    xyz_data_cfg: u8,
    m_ctrl_reg1: u8,
    m_ctrl_reg2: u8,

    /// Advances once per accelerometer burst read to animate activity.
    activity_phase: u32,
    accel: [i16; 3],
    mag: [i16; 3],

    /// Set once `set_sample` is called: latches the accel triplet so the
    /// built-in "grazing cow" animation stops overwriting it on every
    /// burst-read (live UI input must stick, not be animated away).
    manual: bool,

    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Fxos8700 {
    fn default() -> Self {
        Self::new(0x1f)
    }
}

impl Fxos8700 {
    pub fn new(address: u8) -> Self {
        let mut s = Self {
            address,
            current_register: 0,
            register_address_written: false,
            ctrl_reg1: 0,
            ctrl_reg2: 0,
            xyz_data_cfg: 0,
            m_ctrl_reg1: 0,
            m_ctrl_reg2: 0,
            activity_phase: 0,
            accel: [0, 0, ONE_G_LJ],
            mag: [0, 0, 0],
            manual: false,
            component_id: None,
        };
        s.refresh_pose();
        s
    }

    /// Deterministic "grazing cow" motion: the tag is roughly upright (Z ≈ +1 g)
    /// while the head sways as the animal moves and ruminates. No RNG — the
    /// pattern is a fixed walk over `activity_phase` so traces are reproducible.
    fn refresh_pose(&mut self) {
        // Small triangle waves on X/Y, a few hundred milli-g, out of phase.
        let p = (self.activity_phase % 8) as i32;
        let tri = |k: i32| -> i16 {
            // 0,1,2,3,4,3,2,1 → -4..+4 scaled to ~±0.35 g (left-justified).
            let t = if k < 5 { k } else { 8 - k };
            ((t - 2) * 600) as i16
        };
        self.accel[0] = tri(p);
        self.accel[1] = tri((p + 3) % 8);
        self.accel[2] = ONE_G_LJ - (tri(p).abs() / 4); // slight Z dip while moving
                                                       // Magnetometer: a slowly rotating heading vector (earth field-ish).
        self.mag[0] = tri((p + 1) % 8) * 4;
        self.mag[1] = tri((p + 5) % 8) * 4;
        self.mag[2] = 1200;
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            // STATUS: ZYXDR (0x08) + ZYXOW set — data is always ready in sim.
            REG_STATUS => 0xFF,
            // Accelerometer OUT_X/Y/Z MSB:LSB (0x01..0x06), 14-bit left-justified.
            0x01 => (self.accel[0] >> 8) as u8,
            0x02 => (self.accel[0] & 0xFF) as u8,
            0x03 => (self.accel[1] >> 8) as u8,
            0x04 => (self.accel[1] & 0xFF) as u8,
            0x05 => (self.accel[2] >> 8) as u8,
            0x06 => (self.accel[2] & 0xFF) as u8,
            REG_WHO_AM_I => WHOAMI_FXOS8700,
            0x0E => self.xyz_data_cfg,
            0x2A => self.ctrl_reg1,
            0x2B => self.ctrl_reg2,
            // Magnetometer M_OUT_X/Y/Z MSB:LSB (0x33..0x38), 16-bit.
            0x33 => (self.mag[0] >> 8) as u8,
            0x34 => (self.mag[0] & 0xFF) as u8,
            0x35 => (self.mag[1] >> 8) as u8,
            0x36 => (self.mag[1] & 0xFF) as u8,
            0x37 => (self.mag[2] >> 8) as u8,
            0x38 => (self.mag[2] & 0xFF) as u8,
            REG_TEMP => 0x14, // ~+20 °C die temp (0.96 °C/LSB)
            0x5B => self.m_ctrl_reg1,
            0x5C => self.m_ctrl_reg2,
            _ => 0,
        }
    }

    /// Accepts a live sample (e.g. from a UI slider) and latches `manual` so
    /// the built-in "grazing cow" animation in `read()` stops overwriting it.
    pub fn set_sample(&mut self, ax: i16, ay: i16, az: i16) {
        self.accel = [ax, ay, az];
        self.manual = true;
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0x0E => self.xyz_data_cfg = value,
            0x2A => self.ctrl_reg1 = value,
            0x2B => self.ctrl_reg2 = value & !0x40, // RST is self-clearing
            0x5B => self.m_ctrl_reg1 = value,
            0x5C => self.m_ctrl_reg2 = value,
            _ => {}
        }
    }
}

impl I2cDevice for Fxos8700 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        // Advance the activity pose at the start of each accelerometer burst so
        // every sample_fetch sees fresh, believable motion — unless a manual
        // sample has been latched via `set_sample`, in which case the live
        // value must stick rather than be animated away.
        if self.current_register == REG_OUT_X_MSB && !self.manual {
            self.activity_phase = self.activity_phase.wrapping_add(1);
            self.refresh_pose();
        }
        let val = self.read_register(self.current_register);
        // Hybrid auto-increment: once configured, a burst that walks off the end
        // of the accel block (…0x06) jumps straight to the magnetometer block
        // (0x33) so the driver reads accel+mag in one transaction (datasheet
        // §14.2). Only the auto-increment path is remapped; explicit pointer
        // writes are not.
        let autoinc = (self.m_ctrl_reg2 & 0x20) != 0;
        self.current_register = if autoinc && self.current_register == 0x06 {
            REG_M_OUT_X_MSB
        } else {
            self.current_register.wrapping_add(1)
        };
        val
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            self.current_register = data;
            self.register_address_written = true;
        } else {
            self.write_register(self.current_register, data);
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        // End of transaction: the next write is a fresh register pointer. The
        // register pointer itself is preserved across the repeated-START that
        // separates the address-write and data-read phases of a burst read.
        self.register_address_written = false;
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

/// Drivable accelerometer axes, in g. The FXOS8700 accel is 14-bit
/// left-justified: at the ±2 g default full-scale 1 g = `ONE_G_LJ` (0x1000 =
/// 4096) raw counts. The conversion follows the LIVE `xyz_data_cfg` FS bits
/// the firmware wrote (±2/±4/±8 g halve the counts-per-g each step) — the
/// schema range below is the hardware maximum (±8 g); a value beyond the
/// currently configured full-scale saturates, exactly like the silicon.
impl crate::sim_input::SimInput for Fxos8700 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        use crate::sim_input::InputChannel;
        const CH: &[InputChannel] = &[
            InputChannel {
                key: "x",
                label: "X",
                unit: "g",
                min: -8.0,
                max: 8.0,
            },
            InputChannel {
                key: "y",
                label: "Y",
                unit: "g",
                min: -8.0,
                max: 8.0,
            },
            InputChannel {
                key: "z",
                label: "Z",
                unit: "g",
                min: -8.0,
                max: 8.0,
            },
        ];
        CH
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // Live full-scale from xyz_data_cfg FS bits: 0=±2g, 1=±4g, 2=±8g.
        let fs = (self.xyz_data_cfg & 0x03).min(2) as u32;
        let counts_per_g = (ONE_G_LJ as f64) / (1 << fs) as f64;
        let full_scale = (2 << fs) as f64;
        let raw = (value.clamp(-full_scale, full_scale) * counts_per_g).round() as i16;
        let axis = match key {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => unreachable!("require_channel validated the key"),
        };
        self.accel[axis] = raw;
        // Latch manual so the built-in animation in `read()` stops overwriting
        // the driven value — same contract as `set_sample`.
        self.manual = true;
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

    /// Burst-reads one accel axis (MSB then LSB) the way the Zephyr driver
    /// does: write the register pointer, then read the two bytes, then end
    /// the transaction (mirrors the bmp280 tests' write/read/stop shape,
    /// adapted to Fxos8700's `register_address_written` pointer-vs-data gate
    /// which `stop()` resets between transactions).
    fn read_axis(s: &mut Fxos8700, msb_reg: u8) -> i16 {
        s.stop();
        s.write(msb_reg);
        let hi = s.read() as i16;
        let lo = s.read() as i16;
        s.stop();
        (hi << 8) | (lo & 0xFF)
    }

    #[test]
    fn fxos8700_manual_sample_overrides_animation() {
        let mut s = Fxos8700::new(0x1f);
        s.set_sample(0x0800, -0x0400, 0x0100);
        // Burst-read OUT_X_MSB..OUT_X_LSB twice; value must stay put (no animation).
        let x1 = read_axis(&mut s, 0x01);
        let x2 = read_axis(&mut s, 0x01);
        assert_eq!(x1, 0x0800);
        assert_eq!(x2, 0x0800, "manual value must not animate away");
    }
}
