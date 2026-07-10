// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Sensirion **SGP41** VOC + NOx gas sensor as an [`I2cDevice`].
//!
//! The SGP41 returns *raw* signals (`SRAW_VOC`, `SRAW_NOX`) — 16-bit ticks. The
//! human-facing VOC Index (1..500, nominal 100) is computed on the host by
//! Sensirion's Gas Index Algorithm from those raw ticks, so this model emits
//! `SRAW` and the firmware runs the real algorithm on top. That split is exactly
//! how a real SGP41 integration works.
//!
//! Datasheet (SGP41, Sensirion, rev 1.1) protocol — 16-bit big-endian commands,
//! responses are 16-bit words each followed by a CRC-8 (poly 0x31) byte:
//! - `0x2612` execute_conditioning (params RH+T words) → 1 word: SRAW_VOC
//! - `0x2619` measure_raw_signals (params RH+T words) → 2 words: SRAW_VOC, SRAW_NOX
//! - `0x280E` execute_self_test → 1 word: `0xD400` low byte = result (0 = OK)
//! - `0x3615` turn_heater_off (no response)
//! - `0x3682` get_serial_number → 3 words
//!
//! The VOC raw signal advances along a [`Ramp`] each `measure_raw_signals`, so
//! the firmware's VOC Index drifts as the room's air load changes.

use crate::peripherals::components::air_scene::Ramp;
use crate::peripherals::components::sensirion::encode_words;
use crate::peripherals::i2c::I2cDevice;

pub const SGP41_ADDR: u8 = 0x59;

const CMD_EXECUTE_CONDITIONING: u16 = 0x2612;
const CMD_MEASURE_RAW: u16 = 0x2619;
const CMD_EXECUTE_SELF_TEST: u16 = 0x280E;
const CMD_TURN_HEATER_OFF: u16 = 0x3615;
const CMD_GET_SERIAL: u16 = 0x3682;

/// SGP41 model.
pub struct Sgp41 {
    address: u8,
    voc_sraw: Ramp,
    nox_sraw: Ramp,
    write_buf: Vec<u8>,
    read_buf: Vec<u8>,
    read_idx: usize,
}

impl Sgp41 {
    /// `voc_start`/`voc_target` are raw SRAW ticks (~20000..40000 typical);
    /// `nox` is a steady NOx raw tick value; `alpha` the per-measure ramp rate.
    pub fn new(address: u8, voc_start: f64, voc_target: f64, nox: f64, alpha: f64) -> Self {
        let address = if address == 0 { SGP41_ADDR } else { address };
        Self {
            address,
            voc_sraw: Ramp::new(voc_start, voc_target, alpha),
            nox_sraw: Ramp::new(nox, nox, 0.0),
            write_buf: Vec::with_capacity(8),
            read_buf: Vec::new(),
            read_idx: 0,
        }
    }

    pub fn new_default(address: u8) -> Self {
        Self::new(address, 28000.0, 34000.0, 16000.0, 0.08)
    }

    fn dispatch(&mut self, cmd: u16) {
        self.read_buf.clear();
        self.read_idx = 0;
        match cmd {
            CMD_EXECUTE_CONDITIONING => {
                // Conditioning returns only the VOC raw signal; don't advance —
                // it runs before measurement starts.
                let voc = self.voc_sraw.value().round().clamp(0.0, 65535.0) as u16;
                self.read_buf = encode_words(&[voc]);
            }
            CMD_MEASURE_RAW => {
                let voc = self.voc_sraw.advance().round().clamp(0.0, 65535.0) as u16;
                let nox = self.nox_sraw.advance().round().clamp(0.0, 65535.0) as u16;
                self.read_buf = encode_words(&[voc, nox]);
            }
            CMD_EXECUTE_SELF_TEST => {
                // Low byte = 0x00 ⇒ all tests passed (datasheet §4.5).
                self.read_buf = encode_words(&[0xD400]);
            }
            CMD_GET_SERIAL => {
                self.read_buf = encode_words(&[0x5347, 0x5034, 0x0031]); // "SGP4" + tag
            }
            CMD_TURN_HEATER_OFF => {}
            _ => {}
        }
    }
}

impl I2cDevice for Sgp41 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.write_buf.clear();
        self.read_idx = 0;
    }

    fn stop(&mut self) {
        // Sensirion command/read are separate transactions; clear the command
        // accumulator at transaction end so the next command dispatches.
        self.write_buf.clear();
    }

    fn write(&mut self, data: u8) {
        self.write_buf.push(data);
        // Command completes on its second byte; the RH/T parameter words that
        // follow `measure_raw_signals` are accepted but don't change the model.
        if self.write_buf.len() == 2 {
            let cmd = ((self.write_buf[0] as u16) << 8) | (self.write_buf[1] as u16);
            self.dispatch(cmd);
        }
    }

    fn read(&mut self) -> u8 {
        let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
        self.read_idx += 1;
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

pub struct Sgp41Kit;
pub static SGP41_KIT: Sgp41Kit = Sgp41Kit;

static SGP41_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "sgp41",
    label: "Sensirion SGP41 VOC/NOx",
    summary: "MOx gas sensor returning raw VOC/NOx signals over I2C.",
    detail: "Sensirion SGP41 at fixed address 0x59, speaking the real Sensirion \
             command protocol with CRC-8 (poly 0x31). Returns raw SRAW_VOC / SRAW_NOX \
             ticks that the on-host Sensirion Gas Index Algorithm converts to a VOC \
             Index; the raw VOC signal advances along a configurable ramp.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to the SGP41 fixed address 0x59.",
        },
        ConfigKey {
            name: "voc_sraw_start",
            ty: ConfigType::Float,
            doc: "Raw VOC tick at the first measurement (~20000..40000). Default 28000.",
        },
        ConfigKey {
            name: "voc_sraw_target",
            ty: ConfigType::Float,
            doc: "Raw VOC tick the ramp approaches. Default 34000.",
        },
        ConfigKey {
            name: "nox_sraw",
            ty: ConfigType::Float,
            doc: "Steady raw NOx tick value. Default 16000.",
        },
        ConfigKey {
            name: "ramp_alpha",
            ty: ConfigType::Float,
            doc: "Per-measurement approach rate 0..1 (0 = flat scene). Default 0.08.",
        },
    ],
    labs: &[LabRef {
        board_id: "leo-airquality-lab",
        chip: "esp32c3",
        example_dir: "esp32c3-leo-airquality",
        demo_elf: "demo-esp32c3-leo-airquality.elf",
    }],
};

impl PeripheralKit for Sgp41Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SGP41_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(SGP41_ADDR)?;
        let voc_start = ctx.config_f64("voc_sraw_start").unwrap_or(28000.0);
        let voc_target = ctx.config_f64("voc_sraw_target").unwrap_or(34000.0);
        let nox = ctx.config_f64("nox_sraw").unwrap_or(16000.0);
        let alpha = ctx.config_f64("ramp_alpha").unwrap_or(0.08);
        ctx.attach_i2c_device(Box::new(Sgp41::new(
            address, voc_start, voc_target, nox, alpha,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::sensirion::crc8;

    fn send_cmd(d: &mut Sgp41, cmd: u16) {
        d.start();
        d.write((cmd >> 8) as u8);
        d.write((cmd & 0xFF) as u8);
    }

    fn read_n(d: &mut Sgp41, n: usize) -> Vec<u8> {
        d.start();
        (0..n).map(|_| d.read()).collect()
    }

    #[test]
    fn address_defaults_to_0x59() {
        assert_eq!(Sgp41::new_default(0).address(), 0x59);
    }

    #[test]
    fn measure_raw_returns_two_words_with_valid_crcs() {
        let mut d = Sgp41::new_default(SGP41_ADDR);
        send_cmd(&mut d, CMD_MEASURE_RAW);
        let b = read_n(&mut d, 6);
        assert_eq!(b.len(), 6);
        for chunk in b.chunks(3) {
            assert_eq!(chunk[2], crc8(&chunk[..2]));
        }
    }

    #[test]
    fn voc_raw_climbs_with_air_load() {
        let mut d = Sgp41::new_default(SGP41_ADDR);
        let mut first = 0u16;
        let mut last = 0u16;
        for i in 0..60 {
            send_cmd(&mut d, CMD_MEASURE_RAW);
            let b = read_n(&mut d, 6);
            let voc = ((b[0] as u16) << 8) | b[1] as u16;
            if i == 0 {
                first = voc;
            }
            last = voc;
        }
        assert!(last > first, "VOC raw should rise: {first} -> {last}");
    }

    #[test]
    fn self_test_passes() {
        let mut d = Sgp41::new_default(SGP41_ADDR);
        send_cmd(&mut d, CMD_EXECUTE_SELF_TEST);
        let b = read_n(&mut d, 3);
        assert_eq!(b[1], 0x00, "self-test low byte 0 = pass");
        assert_eq!(b[2], crc8(&b[..2]));
    }

    #[test]
    fn conditioning_uses_real_command_0x2612() {
        // The real Sensirion execute_conditioning command is 0x2612 and returns
        // one SRAW_VOC word; an unhandled command would return 0xFF (CRC fail).
        assert_eq!(CMD_EXECUTE_CONDITIONING, 0x2612);
        let mut d = Sgp41::new_default(SGP41_ADDR);
        send_cmd(&mut d, CMD_EXECUTE_CONDITIONING);
        let b = read_n(&mut d, 3);
        assert_ne!(b[0], 0xFF, "conditioning must return a real VOC word");
        assert_eq!(b[2], crc8(&b[..2]), "valid CRC");
    }
}
