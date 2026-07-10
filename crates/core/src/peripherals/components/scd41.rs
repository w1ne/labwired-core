// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Sensirion **SCD41** CO₂ + temperature + humidity sensor as an [`I2cDevice`].
//!
//! This is the hero of the Leo air-quality board: the unmodified Sensirion
//! SCD4x embedded driver issues these exact commands and decodes the words +
//! CRCs this model clocks back, so the firmware's CO₂ reading is provably real.
//!
//! Datasheet (SCD4x, Sensirion, rev 1.3) protocol — 16-bit big-endian commands,
//! responses are 16-bit words each followed by a CRC-8 (poly 0x31) byte:
//! - `0x21B1` start_periodic_measurement            (no response)
//! - `0x21AC` start_low_power_periodic_measurement  (no response)
//! - `0xE4B8` get_data_ready_status   → 1 word: ready when `word & 0x07FF != 0`
//! - `0xEC05` read_measurement        → 3 words: CO₂ ppm, T raw, RH raw
//! - `0x3F86` stop_periodic_measurement             (no response)
//! - `0x3682` get_serial_number       → 3 words
//! - `0x3646` reinit / `0x3632` factory_reset / `0x36F6` wake_up (no response)
//! - `0x3639` perform_self_test       → 1 word: 0x0000 = no malfunction
//! - `0x219D` measure_single_shot     (write-only trigger; no response)
//!
//! Word encodings (datasheet §3.6.2):
//! - CO₂ [ppm] = `word`
//! - T   [°C]  = `-45 + 175 * word / 65535`
//! - RH  [%]   = `100 * word / 65535`
//!
//! The CO₂ value advances along a [`Ramp`] each `read_measurement`, so a normal
//! scenario climbs from fresh toward stuffy and the firmware verdict flips.

use crate::peripherals::components::air_scene::Ramp;
use crate::peripherals::components::sensirion::encode_words;
use crate::peripherals::i2c::I2cDevice;

pub const SCD41_ADDR: u8 = 0x62;

// Commands (16-bit, big-endian).
const CMD_START_PERIODIC: u16 = 0x21B1;
const CMD_START_LOW_POWER: u16 = 0x21AC;
const CMD_GET_DATA_READY: u16 = 0xE4B8;
const CMD_READ_MEASUREMENT: u16 = 0xEC05;
const CMD_STOP_PERIODIC: u16 = 0x3F86;
const CMD_GET_SERIAL: u16 = 0x3682;
const CMD_PERFORM_SELF_TEST: u16 = 0x3639;

/// SCD41 model.
pub struct Scd41 {
    address: u8,
    co2: Ramp,
    temp_c: Ramp,
    rh: Ramp,
    periodic_running: bool,
    /// Bytes the master has written this transaction (command + params).
    write_buf: Vec<u8>,
    /// Response bytes queued by the last command; drained by `read()`.
    read_buf: Vec<u8>,
    read_idx: usize,
}

impl Scd41 {
    /// Build with explicit scene parameters. `co2_start`/`co2_target` in ppm,
    /// `alpha` the per-measurement ramp rate (0 = flat scene).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        address: u8,
        co2_start: f64,
        co2_target: f64,
        alpha: f64,
        temp_c: f64,
        rh: f64,
        rh_target: f64,
    ) -> Self {
        let address = if address == 0 { SCD41_ADDR } else { address };
        Self {
            address,
            co2: Ramp::new(co2_start, co2_target, alpha),
            // Temperature/humidity drift upward as a room fills; pin them flat if
            // alpha is 0 so a "frozen" scenario stays put. The humidity target is
            // configurable so a damp room can climb into mold-favorable range.
            temp_c: Ramp::new(temp_c, temp_c + 1.5, alpha * 0.5),
            rh: Ramp::new(rh, rh_target, alpha * 0.5),
            periodic_running: false,
            write_buf: Vec::with_capacity(8),
            read_buf: Vec::new(),
            read_idx: 0,
        }
    }

    /// Default fresh→stuffy scenario: 450 → 1400 ppm at 22 °C / 45 → 51 %RH.
    pub fn new_default(address: u8) -> Self {
        Self::new(address, 450.0, 1400.0, 0.08, 22.0, 45.0, 51.0)
    }

    fn encode_temperature(t_c: f64) -> u16 {
        (((t_c + 45.0) / 175.0) * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16
    }

    fn encode_humidity(rh: f64) -> u16 {
        ((rh / 100.0) * 65535.0).round().clamp(0.0, 65535.0) as u16
    }

    /// Dispatch a completed command word, queuing any response bytes.
    fn dispatch(&mut self, cmd: u16) {
        self.read_buf.clear();
        self.read_idx = 0;
        match cmd {
            CMD_START_PERIODIC | CMD_START_LOW_POWER => self.periodic_running = true,
            CMD_STOP_PERIODIC => self.periodic_running = false,
            CMD_GET_DATA_READY => {
                // Non-zero low 11 bits ⇒ data ready. Deterministic always-ready.
                self.read_buf = encode_words(&[0x8006]);
            }
            CMD_READ_MEASUREMENT => {
                let co2 = self.co2.advance().round().clamp(0.0, 40000.0) as u16;
                let t = Self::encode_temperature(self.temp_c.advance());
                let rh = Self::encode_humidity(self.rh.advance());
                self.read_buf = encode_words(&[co2, t, rh]);
            }
            CMD_GET_SERIAL => {
                self.read_buf = encode_words(&[0x4C45, 0x4F31, 0x0041]); // "LEO1" + tag
            }
            CMD_PERFORM_SELF_TEST => {
                // 0x0000 ⇒ no sensor malfunction (datasheet §3.9.3).
                self.read_buf = encode_words(&[0x0000]);
            }
            // measure_single_shot (0x219D) is a write-only trigger: the real
            // driver issues it then reads the result via read_measurement, so it
            // returns no data here — fall through to the no-response arm.
            _ => {} // single_shot, reinit, factory_reset, wake_up, set_* — no response
        }
    }
}

impl I2cDevice for Scd41 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // (Re)START within a transaction: clear the command buffer and rewind
        // the read cursor. Sensirion command and read phases are *separate*
        // transactions, and the C3 controller only calls start() on a repeated
        // START — so the real reset between commands happens in stop().
        self.write_buf.clear();
        self.read_idx = 0;
    }

    fn stop(&mut self) {
        // End of a transaction: clear the command accumulator so the next
        // command transaction starts fresh. (read_idx is rewound by dispatch.)
        self.write_buf.clear();
    }

    fn write(&mut self, data: u8) {
        self.write_buf.push(data);
        // A command completes on its second byte; parameter words (for set_*
        // commands) follow but the model doesn't need them.
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

pub struct Scd41Kit;
pub static SCD41_KIT: Scd41Kit = Scd41Kit;

static SCD41_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "scd41",
    label: "Sensirion SCD41 CO₂",
    summary: "Photoacoustic CO₂ + temperature + humidity sensor over I2C.",
    detail: "Sensirion SCD41 at fixed address 0x62, speaking the real Sensirion \
             command protocol (16-bit commands, 16-bit words + CRC-8 poly 0x31), so \
             the unmodified Sensirion SCD4x vendor driver decodes it on-target. CO₂ \
             advances along a configurable ramp each read_measurement, so a closed \
             room climbs from fresh toward stuffy and the firmware verdict flips.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to the SCD41 fixed address 0x62.",
        },
        ConfigKey {
            name: "co2_start_ppm",
            ty: ConfigType::Float,
            doc: "CO₂ the ramp starts from, ppm (fresh-room baseline). Default 450.",
        },
        ConfigKey {
            name: "co2_target_ppm",
            ty: ConfigType::Float,
            doc: "CO₂ the ramp approaches, ppm (closed-room steady state). Default 1400.",
        },
        ConfigKey {
            name: "ramp_alpha",
            ty: ConfigType::Float,
            doc: "Per-measurement approach rate 0..1 (0 = flat scene). Default 0.08.",
        },
        ConfigKey {
            name: "temp_c",
            ty: ConfigType::Float,
            doc: "Starting temperature, °C. Default 22.0.",
        },
        ConfigKey {
            name: "rh_pct",
            ty: ConfigType::Float,
            doc: "Starting relative humidity, %. Default 45.0.",
        },
        ConfigKey {
            name: "rh_target_pct",
            ty: ConfigType::Float,
            doc: "Steady-state relative humidity the room climbs toward, %. \
                  Default rh_pct + 6. Raise above 60 for a damp, mold-favorable room.",
        },
    ],
    labs: &[LabRef {
        board_id: "esp32c3-leo-airquality",
        chip: "esp32c3",
        example_dir: "esp32c3-leo-airquality",
        demo_elf: "demo-esp32c3-leo-airquality.elf",
    }],
};

impl PeripheralKit for Scd41Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SCD41_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(SCD41_ADDR)?;
        let co2_start = ctx.config_f64("co2_start_ppm").unwrap_or(450.0);
        let co2_target = ctx.config_f64("co2_target_ppm").unwrap_or(1400.0);
        let alpha = ctx.config_f64("ramp_alpha").unwrap_or(0.08);
        let temp_c = ctx.config_f64("temp_c").unwrap_or(22.0);
        let rh = ctx.config_f64("rh_pct").unwrap_or(45.0);
        let rh_target = ctx.config_f64("rh_target_pct").unwrap_or(rh + 6.0);
        ctx.attach_i2c_device(Box::new(Scd41::new(
            address, co2_start, co2_target, alpha, temp_c, rh, rh_target,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::sensirion::crc8;

    fn read_n(d: &mut Scd41, n: usize) -> Vec<u8> {
        d.start();
        (0..n).map(|_| d.read()).collect()
    }

    fn send_cmd(d: &mut Scd41, cmd: u16) {
        d.start();
        d.write((cmd >> 8) as u8);
        d.write((cmd & 0xFF) as u8);
    }

    #[test]
    fn address_defaults_to_0x62() {
        assert_eq!(Scd41::new_default(0).address(), 0x62);
    }

    #[test]
    fn read_measurement_returns_9_bytes_with_valid_crcs() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, CMD_START_PERIODIC);
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let bytes = read_n(&mut d, 9);
        assert_eq!(bytes.len(), 9);
        for chunk in bytes.chunks(3) {
            assert_eq!(chunk[2], crc8(&chunk[..2]), "each word carries a valid CRC");
        }
    }

    #[test]
    fn first_co2_word_is_near_fresh_start() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let b = read_n(&mut d, 9);
        let co2 = ((b[0] as u16) << 8) | b[1] as u16;
        assert!(
            (450..=600).contains(&co2),
            "first read is fresh-ish, got {co2}"
        );
    }

    #[test]
    fn co2_climbs_toward_stuffy_over_many_reads() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        let mut last = 0u16;
        for _ in 0..80 {
            send_cmd(&mut d, CMD_READ_MEASUREMENT);
            let b = read_n(&mut d, 9);
            last = ((b[0] as u16) << 8) | b[1] as u16;
        }
        assert!(last > 1300, "CO₂ should approach 1400 ppm, got {last}");
    }

    #[test]
    fn temperature_decodes_within_room_range() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let b = read_n(&mut d, 9);
        let t_word = ((b[3] as u16) << 8) | b[4] as u16;
        let t_c = -45.0 + 175.0 * (t_word as f64) / 65535.0;
        assert!((18.0..28.0).contains(&t_c), "room temp, got {t_c:.1}");
    }

    #[test]
    fn data_ready_reports_ready() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, CMD_GET_DATA_READY);
        let b = read_n(&mut d, 3);
        let word = ((b[0] as u16) << 8) | b[1] as u16;
        assert_ne!(word & 0x07FF, 0, "data-ready word must be non-zero");
        assert_eq!(b[2], crc8(&b[..2]));
    }

    #[test]
    fn self_test_reports_no_malfunction() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, CMD_PERFORM_SELF_TEST);
        let b = read_n(&mut d, 3);
        let word = ((b[0] as u16) << 8) | b[1] as u16;
        assert_eq!(word, 0x0000, "0x0000 = no malfunction");
        assert_eq!(b[2], crc8(&b[..2]));
    }

    #[test]
    fn single_shot_is_write_only_trigger() {
        // measure_single_shot (0x219D) must NOT queue a measurement response and
        // must NOT advance the scene — the driver reads the result via 0xEC05.
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, 0x219D);
        let b = read_n(&mut d, 9);
        assert!(
            b.iter().all(|&x| x == 0xFF),
            "no response queued for single-shot"
        );
        // The next real read_measurement should still start near the fresh value.
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let m = read_n(&mut d, 9);
        let co2 = ((m[0] as u16) << 8) | m[1] as u16;
        assert!(
            (450..=600).contains(&co2),
            "scene not advanced by single-shot: {co2}"
        );
    }

    #[test]
    fn flat_scenario_holds_co2() {
        let mut d = Scd41::new(SCD41_ADDR, 800.0, 800.0, 0.0, 22.0, 45.0, 45.0);
        let mut seen = vec![];
        for _ in 0..5 {
            send_cmd(&mut d, CMD_READ_MEASUREMENT);
            let b = read_n(&mut d, 9);
            seen.push(((b[0] as u16) << 8) | b[1] as u16);
        }
        assert!(
            seen.iter().all(|&v| v == 800),
            "flat scene holds 800 ppm: {seen:?}"
        );
    }
}
