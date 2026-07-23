// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Vishay **VEML7700** ambient-light sensor as an [`I2cDevice`].
//!
//! **Status: byte-parity oracle only.** The shipping VEML7700 is now the
//! declarative descriptor `configs/devices/veml7700.yaml`, driven by
//! [`super::declarative_i2c::GenericI2cDevice`] and registered as
//! [`super::declarative_i2c::VEML7700_KIT`]. This hand-written model is retained
//! solely as the reference the parity test (`super::veml7700_parity`) proves the
//! declarative device byte-identical against — hence the whole module is
//! `#[cfg(test)]`. It is no longer a registry kit.
//!
//! Unlike the Sensirion parts on this board, the VEML7700 is a classic
//! command-register device: the master writes a 1-byte register pointer, then
//! reads/writes a **16-bit little-endian** word (low byte first).
//!
//! Datasheet (VEML7700, Vishay, rev 1.6) register map:
//! - `0x00` ALS_CONF   (config: gain, integration time, power)
//! - `0x01` ALS_WH / `0x02` ALS_WL (window thresholds)
//! - `0x03` PSM        (power-saving mode)
//! - `0x04` ALS        (ambient-light output, 16-bit) — read
//! - `0x05` WHITE      (white-channel output, 16-bit) — read
//! - `0x06` ALS_INT    (interrupt status, 16-bit) — read
//!
//! Lux = `raw_counts × resolution`. Resolution scales inversely with the
//! programmed gain and integration time, anchored at `0.0576 lux/count` for the
//! power-on default (gain ×1, integration time 100 ms — ALS_CONF reset value
//! 0x0000). The model honours ALS_CONF gain/IT bits when converting lux to raw
//! counts, so firmware that reprograms gain/IT sees the same count scaling a
//! real part would.
//!
//! The illuminance is an externally driven variable: it changes only when
//! something drives it through the ONE stimulus contract,
//! [`crate::sim_input::SimInput`] (channel `lux`). Config seeds its initial
//! value.

use crate::peripherals::i2c::I2cDevice;

pub const VEML7700_ADDR: u8 = 0x10;

/// Lux per count at the power-on default gain ×1 / integration-time 100 ms.
const RESOLUTION_LUX_PER_COUNT: f64 = 0.0576;

const REG_ALS_CONF: u8 = 0x00;
const REG_ALS: u8 = 0x04;
const REG_WHITE: u8 = 0x05;

/// VEML7700 model.
pub struct Veml7700 {
    address: u8,
    /// Illuminance the part reports, lux. Externally driven (see `SimInput`).
    lux: f64,
    /// Config registers the firmware writes (ALS_CONF etc.), echoed back on read.
    conf: [u16; 4],
    /// Selected register pointer for the current transaction.
    pointer: u8,
    /// Bytes written this transaction (pointer + optional data low/high).
    write_buf: Vec<u8>,
    /// Latched 16-bit read word (LE) for the current read, and which byte is next.
    read_word: u16,
    read_byte_idx: usize,
    /// Whether the current read has latched its word yet.
    latched: bool,
    /// system.yaml `external_devices` id, stamped at attach.
    component_id: Option<String>,
}

impl Veml7700 {
    /// `lux` is the initial illuminance the part reports.
    pub fn new(address: u8, lux: f64) -> Self {
        let address = if address == 0 { VEML7700_ADDR } else { address };
        Self {
            address,
            lux,
            conf: [0; 4],
            pointer: 0,
            write_buf: Vec::with_capacity(4),
            read_word: 0,
            read_byte_idx: 0,
            latched: false,
            component_id: None,
        }
    }

    pub fn new_default(address: u8) -> Self {
        Self::new(address, 450.0)
    }

    /// Effective lux/count from the programmed ALS_CONF gain + integration time.
    /// Resolution scales inversely with both, anchored at 0.0576 lux/count for
    /// gain ×1 / 100 ms (Vishay app-note resolution table). ALS_CONF bit layout:
    /// gain in bits [12:11], integration time in bits [9:6].
    fn resolution(&self) -> f64 {
        let conf = self.conf[REG_ALS_CONF as usize];
        let it_ms = match (conf >> 6) & 0xF {
            0b1100 => 25.0,
            0b1000 => 50.0,
            0b0000 => 100.0,
            0b0001 => 200.0,
            0b0010 => 400.0,
            0b0011 => 800.0,
            _ => 100.0, // reserved encodings → treat as the 100 ms default
        };
        let gain_factor = match (conf >> 11) & 0x3 {
            0b00 => 1.0,
            0b01 => 2.0,
            0b10 => 0.125, // ×1/8
            0b11 => 0.25,  // ×1/4
            _ => 1.0,
        };
        RESOLUTION_LUX_PER_COUNT * (100.0 / it_ms) / gain_factor
    }

    fn lux_to_counts(&self, lux: f64) -> u16 {
        (lux / self.resolution()).round().clamp(0.0, 65535.0) as u16
    }

    /// Latch the word a read of the current pointer returns.
    fn latch_read_word(&mut self) {
        self.read_word = match self.pointer {
            REG_ALS => self.lux_to_counts(self.lux),
            // White channel runs a bit brighter than the visible ALS channel.
            REG_WHITE => self.lux_to_counts(self.lux * 1.15),
            r if (r as usize) < self.conf.len() => self.conf[r as usize],
            _ => 0,
        };
        self.latched = true;
    }
}

impl I2cDevice for Veml7700 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // (Re)START frames either a write phase (pointer set) or the read phase.
        // Rewind the read cursor and clear the latch so the next read re-latches
        // the current value; keep the pointer set by the preceding write.
        self.write_buf.clear();
        self.read_byte_idx = 0;
        self.latched = false;
    }

    fn stop(&mut self) {
        // The C3 controller only calls start() on a repeated START, so clear the
        // command accumulator at transaction end too — otherwise a config-write
        // transaction leaves stale bytes that corrupt the next register pointer.
        self.write_buf.clear();
    }

    fn write(&mut self, data: u8) {
        self.write_buf.push(data);
        match self.write_buf.len() {
            1 => self.pointer = data,
            // 16-bit LE config write: low byte then high byte.
            3 if (self.pointer as usize) < self.conf.len() => {
                let lo = self.write_buf[1] as u16;
                let hi = self.write_buf[2] as u16;
                self.conf[self.pointer as usize] = (hi << 8) | lo;
            }
            _ => {}
        }
    }

    fn read(&mut self) -> u8 {
        // Latch the value on the first byte of the read, then stream it
        // little-endian: low byte first, then high byte.
        if !self.latched {
            self.latch_read_word();
        }
        let byte = match self.read_byte_idx {
            0 => (self.read_word & 0xFF) as u8,
            1 => (self.read_word >> 8) as u8,
            _ => 0xFF,
        };
        self.read_byte_idx += 1;
        byte
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

/// Drivable illuminance, in lux. 120 000 lx is the VEML7700's maximum
/// detectable range (gain ×1/8, IT 25 ms). ONE table backs BOTH the `SimInput`
/// impl and the kit metadata.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[crate::sim_input::InputChannel {
    key: "lux",
    label: "Illuminance",
    unit: "lx",
    min: 0.0,
    max: 120000.0,
}];

impl crate::sim_input::SimInput for Veml7700 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.lux = value;
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// The `PeripheralKit` registration that once lived here now lives in the
// declarative descriptor `configs/devices/veml7700.yaml` +
// `super::declarative_i2c::VEML7700_KIT`. All of that metadata (label, summary,
// detail, config_keys, the Leo lab, and the lux input) moved verbatim into the
// YAML `metadata:` block, so the offline peripherals manifest is unchanged.

#[cfg(test)]
mod tests {
    use super::*;

    /// Point at a register and read its 16-bit LE word.
    fn read_reg(d: &mut Veml7700, reg: u8) -> u16 {
        d.start();
        d.write(reg);
        d.start(); // repeated START into the read phase
        let lo = d.read() as u16;
        let hi = d.read() as u16;
        (hi << 8) | lo
    }

    #[test]
    fn address_defaults_to_0x10() {
        assert_eq!(Veml7700::new_default(0).address(), 0x10);
    }

    #[test]
    fn als_reads_back_as_plausible_lux() {
        let mut d = Veml7700::new_default(VEML7700_ADDR);
        let counts = read_reg(&mut d, REG_ALS);
        let lux = counts as f64 * RESOLUTION_LUX_PER_COUNT;
        assert!(
            (300.0..600.0).contains(&lux),
            "first read is bright-ish: {lux:.0}"
        );
    }

    #[test]
    fn light_holds_until_driven() {
        use crate::sim_input::SimInput;
        let mut d = Veml7700::new_default(VEML7700_ADDR);
        for _ in 0..20 {
            let lux = read_reg(&mut d, REG_ALS) as f64 * RESOLUTION_LUX_PER_COUNT;
            assert!((lux - 450.0).abs() < 0.1, "no self-running scene: {lux:.1}");
        }
        d.set_input("lux", 90.0).unwrap();
        let lux = read_reg(&mut d, REG_ALS) as f64 * RESOLUTION_LUX_PER_COUNT;
        assert!((lux - 90.0).abs() < 0.1, "driven to 90 lux: {lux:.1}");
    }

    #[test]
    fn out_of_range_input_is_rejected() {
        use crate::sim_input::SimInput;
        let mut d = Veml7700::new_default(VEML7700_ADDR);
        assert!(d.set_input("lux", -1.0).is_err());
        assert!(d.set_input("brightness", 10.0).is_err());
    }

    #[test]
    fn config_write_is_read_back() {
        let mut d = Veml7700::new_default(VEML7700_ADDR);
        // Write ALS_CONF = 0x1234 (LE: 0x34, 0x12).
        d.start();
        d.write(0x00);
        d.write(0x34);
        d.write(0x12);
        let v = read_reg(&mut d, 0x00);
        assert_eq!(v, 0x1234, "config register round-trips");
    }

    #[test]
    fn gain_and_integration_time_scale_resolution() {
        // A steady 100 lux so the ALS count depends only on resolution.
        let mut d = Veml7700::new(VEML7700_ADDR, 100.0);
        let default_counts = read_reg(&mut d, REG_ALS); // gain ×1 / IT 100 ms
                                                        // Program ALS_CONF = 0x08C0: gain ×2 (bits[12:11]=01) + IT 800 ms
                                                        // (bits[9:6]=0011) → resolution 0.0036 lux/count → ~16× the counts.
        d.start();
        d.write(REG_ALS_CONF);
        d.write(0xC0); // low byte
        d.write(0x08); // high byte
        let hi_res_counts = read_reg(&mut d, REG_ALS);
        let ratio = hi_res_counts as f64 / default_counts as f64;
        assert!(
            (ratio - 16.0).abs() < 0.5,
            "gain×2 / IT 800 ms should give ~16× counts, got {ratio:.1}"
        );
    }
}
