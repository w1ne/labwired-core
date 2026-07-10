// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Vishay **VEML7700** ambient-light sensor as an [`I2cDevice`].
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
//! 0x0000). The model honours ALS_CONF gain/IT bits when converting its lux
//! [`Ramp`] to raw counts, so firmware that reprograms gain/IT sees the same
//! count scaling a real part would. The default scenario *dims* (450 → 90 lux)
//! as the room settles for the evening, complementing the rising CO₂ story.

use crate::peripherals::components::air_scene::Ramp;
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
    lux: Ramp,
    /// Config registers the firmware writes (ALS_CONF etc.), echoed back on read.
    conf: [u16; 4],
    /// Selected register pointer for the current transaction.
    pointer: u8,
    /// Bytes written this transaction (pointer + optional data low/high).
    write_buf: Vec<u8>,
    /// Latched 16-bit read word (LE) for the current read, and which byte is next.
    read_word: u16,
    read_byte_idx: usize,
    /// Whether the current read has latched its word yet (advance exactly once).
    latched: bool,
}

impl Veml7700 {
    /// `lux_start`/`lux_target` in lux; `alpha` per-read ramp rate (light dims
    /// when `target < start`).
    pub fn new(address: u8, lux_start: f64, lux_target: f64, alpha: f64) -> Self {
        let address = if address == 0 { VEML7700_ADDR } else { address };
        Self {
            address,
            lux: Ramp::new(lux_start, lux_target, alpha),
            conf: [0; 4],
            pointer: 0,
            write_buf: Vec::with_capacity(4),
            read_word: 0,
            read_byte_idx: 0,
            latched: false,
        }
    }

    pub fn new_default(address: u8) -> Self {
        Self::new(address, 450.0, 90.0, 0.08)
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

    /// Latch the word a read of the current pointer returns. Advances the light
    /// ramp exactly once per read transaction (only for the ALS channel).
    fn latch_read_word(&mut self) {
        self.read_word = match self.pointer {
            REG_ALS => {
                let lux = self.lux.advance();
                self.lux_to_counts(lux)
            }
            // White channel runs a bit brighter than ALS; don't advance again.
            REG_WHITE => {
                let lux = self.lux.value() * 1.15;
                self.lux_to_counts(lux)
            }
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
        // Rewind the read cursor and clear the latch so the next read advances
        // the ramp once; keep the pointer set by the preceding write.
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
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Veml7700Kit;
pub static VEML7700_KIT: Veml7700Kit = Veml7700Kit;

static VEML7700_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "veml7700",
    label: "Vishay VEML7700 Light",
    summary: "Ambient-light sensor (lux) over I2C.",
    detail: "Vishay VEML7700 at fixed address 0x10, a register-pointer device with \
             16-bit little-endian words. Reports a raw ALS count the firmware scales \
             to lux at the default gain ×1 / 100 ms resolution (0.0576 lux/count). \
             Light follows a configurable ramp (dims toward evening by default).",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to the VEML7700 fixed address 0x10.",
        },
        ConfigKey {
            name: "lux_start",
            ty: ConfigType::Float,
            doc: "Ambient light at the first reading, lux (daylit room). Default 450.",
        },
        ConfigKey {
            name: "lux_target",
            ty: ConfigType::Float,
            doc: "Lux the ramp approaches (dims when below start). Default 90.",
        },
        ConfigKey {
            name: "ramp_alpha",
            ty: ConfigType::Float,
            doc: "Per-read approach rate 0..1 (0 = flat scene). Default 0.08.",
        },
    ],
    labs: &[LabRef {
        board_id: "esp32c3-leo-airquality",
        chip: "esp32c3",
        example_dir: "esp32c3-leo-airquality",
        demo_elf: "demo-esp32c3-leo-airquality.elf",
    }],
};

impl PeripheralKit for Veml7700Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &VEML7700_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(VEML7700_ADDR)?;
        let lux_start = ctx.config_f64("lux_start").unwrap_or(450.0);
        let lux_target = ctx.config_f64("lux_target").unwrap_or(90.0);
        let alpha = ctx.config_f64("ramp_alpha").unwrap_or(0.08);
        ctx.attach_i2c_device(Box::new(Veml7700::new(
            address, lux_start, lux_target, alpha,
        )))
    }
}

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
    fn light_dims_over_reads() {
        let mut d = Veml7700::new_default(VEML7700_ADDR);
        let mut first = 0.0;
        let mut last = 0.0;
        for i in 0..60 {
            let counts = read_reg(&mut d, REG_ALS);
            let lux = counts as f64 * RESOLUTION_LUX_PER_COUNT;
            if i == 0 {
                first = lux;
            }
            last = lux;
        }
        assert!(last < first, "room dims: {first:.0} -> {last:.0} lux");
        assert!(last < 150.0, "settles toward 90 lux: {last:.0}");
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
        // Flat 100 lux scene so the ALS count depends only on resolution.
        let mut d = Veml7700::new(VEML7700_ADDR, 100.0, 100.0, 0.0);
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
