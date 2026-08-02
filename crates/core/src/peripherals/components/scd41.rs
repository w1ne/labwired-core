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
//! The reported CO₂ / temperature / humidity are externally driven variables:
//! they change only when something drives them through the ONE stimulus
//! contract, [`crate::sim_input::SimInput`] (`co2`, `temperature`, `humidity`)
//! — test-script `stimuli:`, the MCP, or the WASM bridge. The config block only
//! seeds the initial value.

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
    /// CO₂ the part reports, ppm. Externally driven (see `SimInput`).
    co2_ppm: f64,
    /// Air temperature the part reports, °C. Externally driven.
    temp_c: f64,
    /// Relative humidity the part reports, %RH. Externally driven.
    rh_pct: f64,
    periodic_running: bool,
    /// Bytes the master has written this transaction (command + params).
    write_buf: Vec<u8>,
    /// Response bytes queued by the last command; drained by `read()`.
    read_buf: Vec<u8>,
    read_idx: usize,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Scd41 {
    /// Build with the initial values the part reports until something drives
    /// it. `co2_ppm` in ppm, `temp_c` in °C, `rh_pct` in %RH.
    pub fn new(address: u8, co2_ppm: f64, temp_c: f64, rh_pct: f64) -> Self {
        let address = if address == 0 { SCD41_ADDR } else { address };
        Self {
            address,
            co2_ppm,
            temp_c,
            rh_pct,
            periodic_running: false,
            write_buf: Vec::with_capacity(8),
            read_buf: Vec::new(),
            read_idx: 0,
            component_id: None,
        }
    }

    /// Plausible fresh-room defaults: 450 ppm at 22 °C / 45 %RH.
    pub fn new_default(address: u8) -> Self {
        Self::new(address, 450.0, 22.0, 45.0)
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
                let co2 = self.co2_ppm.round().clamp(0.0, 40000.0) as u16;
                let t = Self::encode_temperature(self.temp_c);
                let rh = Self::encode_humidity(self.rh_pct);
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

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Drivable channels, in real engineering units. ONE table backs BOTH the
/// `SimInput` impl and the kit metadata, so the device schema and the runtime
/// API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "co2",
        label: "CO₂",
        unit: "ppm",
        // Datasheet output range for the SCD41 (§1.1): 400..5000 ppm specified,
        // 0..40000 ppm reportable over the 16-bit word.
        min: 0.0,
        max: 40000.0,
    },
    crate::sim_input::InputChannel {
        key: "temperature",
        label: "Temperature",
        unit: "°C",
        // The word encoding spans -45..130 °C exactly.
        min: -45.0,
        max: 130.0,
    },
    crate::sim_input::InputChannel {
        key: "humidity",
        label: "Humidity",
        unit: "%RH",
        min: 0.0,
        max: 100.0,
    },
];

impl crate::sim_input::SimInput for Scd41 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "co2" => self.co2_ppm = value,
            "temperature" => self.temp_c = value,
            "humidity" => self.rh_pct = value,
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

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct Scd41Kit;
pub static SCD41_KIT: Scd41Kit = Scd41Kit;

static SCD41_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "scd41",
    label: "Sensirion SCD41 CO₂",
    summary: "Photoacoustic CO₂ + temperature + humidity sensor over I2C.",
    detail: "Sensirion SCD41 at fixed address 0x62, speaking the real Sensirion \
             command protocol (16-bit commands, 16-bit words + CRC-8 poly 0x31), so \
             the unmodified Sensirion SCD4x vendor driver decodes it on-target. The \
             reported CO₂, temperature and humidity are externally driven inputs \
             (channels co2 / temperature / humidity); config only seeds their initial \
             values.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to the SCD41 fixed address 0x62.",
        },
        ConfigKey {
            name: "co2_ppm",
            ty: ConfigType::Float,
            doc: "Initial CO₂ the part reports, ppm. Default 450. Drive it at runtime \
                  with the `co2` input channel.",
        },
        ConfigKey {
            name: "temp_c",
            ty: ConfigType::Float,
            doc: "Initial temperature, °C. Default 22.0. Runtime channel: `temperature`.",
        },
        ConfigKey {
            name: "rh_pct",
            ty: ConfigType::Float,
            doc: "Initial relative humidity, %. Default 45.0. Runtime channel: `humidity`.",
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
        let co2_ppm = ctx.config_f64("co2_ppm").unwrap_or(450.0);
        let temp_c = ctx.config_f64("temp_c").unwrap_or(22.0);
        let rh_pct = ctx.config_f64("rh_pct").unwrap_or(45.0);
        ctx.attach_i2c_device(Box::new(Scd41::new(address, co2_ppm, temp_c, rh_pct)))
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
    fn first_co2_word_is_the_seeded_value() {
        let mut d = Scd41::new_default(SCD41_ADDR);
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let b = read_n(&mut d, 9);
        let co2 = ((b[0] as u16) << 8) | b[1] as u16;
        assert_eq!(co2, 450, "reports the seeded value, got {co2}");
    }

    #[test]
    fn co2_holds_until_something_drives_it() {
        // No self-running scene: repeated reads return the same value. The ONLY
        // way the number moves is set_input.
        use crate::sim_input::SimInput;
        let mut d = Scd41::new_default(SCD41_ADDR);
        for _ in 0..20 {
            send_cmd(&mut d, CMD_READ_MEASUREMENT);
            let b = read_n(&mut d, 9);
            assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 450);
        }
        d.set_input("co2", 1400.0).unwrap();
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let b = read_n(&mut d, 9);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 1400);
    }

    #[test]
    fn driven_temperature_and_humidity_land_in_the_words() {
        use crate::sim_input::SimInput;
        let mut d = Scd41::new_default(SCD41_ADDR);
        d.set_input("temperature", 31.5).unwrap();
        d.set_input("humidity", 72.0).unwrap();
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let b = read_n(&mut d, 9);
        let t_word = ((b[3] as u16) << 8) | b[4] as u16;
        let rh_word = ((b[6] as u16) << 8) | b[7] as u16;
        let t_c = -45.0 + 175.0 * (t_word as f64) / 65535.0;
        let rh = 100.0 * (rh_word as f64) / 65535.0;
        assert!((t_c - 31.5).abs() < 0.01, "decoded {t_c:.3} °C");
        assert!((rh - 72.0).abs() < 0.01, "decoded {rh:.3} %RH");
    }

    #[test]
    fn out_of_range_input_is_rejected() {
        use crate::sim_input::SimInput;
        let mut d = Scd41::new_default(SCD41_ADDR);
        assert!(d.set_input("humidity", 150.0).is_err());
        assert!(d.set_input("nope", 1.0).is_err());
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
        // The next real read_measurement still reports the seeded value.
        send_cmd(&mut d, CMD_READ_MEASUREMENT);
        let m = read_n(&mut d, 9);
        let co2 = ((m[0] as u16) << 8) | m[1] as u16;
        assert_eq!(co2, 450, "single-shot must not perturb the value: {co2}");
    }

    #[test]
    fn seeded_config_values_are_reported_verbatim() {
        let mut d = Scd41::new(SCD41_ADDR, 800.0, 22.0, 45.0);
        let mut seen = vec![];
        for _ in 0..5 {
            send_cmd(&mut d, CMD_READ_MEASUREMENT);
            let b = read_n(&mut d, 9);
            seen.push(((b[0] as u16) << 8) | b[1] as u16);
        }
        assert!(seen.iter().all(|&v| v == 800), "holds 800 ppm: {seen:?}");
    }
}
