// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Sensirion **SPS30** particulate-matter sensor as an [`I2cDevice`].
//!
//! The SPS30 reports mass and number concentrations across particle-size bins.
//! Datasheet (SPS30, Sensirion, rev 1.0) protocol — 16-bit big-endian commands,
//! responses are 16-bit words each followed by a CRC-8 (poly 0x31) byte:
//! - `0x0010` start_measurement  (param word: `0x0300` float / `0x0500` uint16)
//! - `0x0104` stop_measurement   (no response)
//! - `0x0202` read_data_ready    → 1 word: ready when low byte = 0x01
//! - `0x0300` read_measured_values → 10 values (float mode: each value is 2
//!   words/4 bytes + 2 CRCs; uint16 mode: each value is 1 word + CRC). Order:
//!   mass PM1.0/2.5/4.0/10.0, number PM0.5/1.0/2.5/4.0/10.0, typical size.
//! - `0x1103` wake / `0x1001` sleep / `0x5607` start_fan_cleaning / `0xD304`
//!   device_reset / `0xD210` clear_device_status (no response)
//! - `0xD033` read_serial_number → 48 bytes (16 words)
//! - `0xD002` read_product_type → 12 bytes / `0xD100` read_version → 1 word /
//!   `0xD206` read_device_status_register → 2 words / `0x8004`
//!   read_auto_cleaning_interval → 2 words
//!
//! The four mass concentrations are externally driven variables — they change
//! only when something drives them through the ONE stimulus contract,
//! [`crate::sim_input::SimInput`] (`pm1_0`, `pm2_5`, `pm4_0`, `pm10`). The
//! number concentrations and the typical particle size stay derived from PM2.5,
//! as the model has always done. Config seeds the initial mass distribution.

use crate::peripherals::components::sensirion::{crc8, encode_words};
use crate::peripherals::i2c::I2cDevice;

pub const SPS30_ADDR: u8 = 0x69;

const CMD_START_MEASUREMENT: u16 = 0x0010;
const CMD_STOP_MEASUREMENT: u16 = 0x0104;
const CMD_READ_DATA_READY: u16 = 0x0202;
const CMD_READ_MEASURED_VALUES: u16 = 0x0300;
const CMD_GET_SERIAL: u16 = 0xD033;
const CMD_READ_PRODUCT_TYPE: u16 = 0xD002;
const CMD_READ_VERSION: u16 = 0xD100;
const CMD_READ_STATUS_REGISTER: u16 = 0xD206;
const CMD_READ_AUTO_CLEAN_INTERVAL: u16 = 0x8004;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Float,
    Uint16,
}

/// SPS30 model.
/// Bin ratios of a typical urban aerosol, relative to PM2.5. Used to seed the
/// other mass bins from a single `pm2_5` config value; each bin is independently
/// drivable afterwards.
const PM1_0_RATIO: f64 = 0.78;
const PM4_0_RATIO: f64 = 1.10;
const PM10_RATIO: f64 = 1.18;

/// SPS30 model.
pub struct Sps30 {
    address: u8,
    /// Mass concentrations in µg/m³. Externally driven (see `SimInput`).
    pm1_0: f64,
    pm2_5: f64,
    pm4_0: f64,
    pm10: f64,
    mode: Mode,
    running: bool,
    write_buf: Vec<u8>,
    read_buf: Vec<u8>,
    read_idx: usize,
    /// system.yaml `external_devices` id, stamped at attach.
    component_id: Option<String>,
}

impl Sps30 {
    /// Seed the mass bins from a PM2.5 value in µg/m³, using the typical
    /// urban-aerosol size distribution for the other bins.
    pub fn new(address: u8, pm2_5: f64) -> Self {
        let address = if address == 0 { SPS30_ADDR } else { address };
        Self {
            address,
            pm1_0: pm2_5 * PM1_0_RATIO,
            pm2_5,
            pm4_0: pm2_5 * PM4_0_RATIO,
            pm10: pm2_5 * PM10_RATIO,
            mode: Mode::Float,
            running: false,
            write_buf: Vec::with_capacity(8),
            read_buf: Vec::new(),
            read_idx: 0,
            component_id: None,
        }
    }

    pub fn new_default(address: u8) -> Self {
        Self::new(address, 6.0)
    }

    /// Encode one IEEE-754 float as the SPS30 clocks it: big-endian, split into
    /// two 16-bit words, each followed by its CRC.
    fn push_float(buf: &mut Vec<u8>, value: f32) {
        let b = value.to_be_bytes();
        for word in [[b[0], b[1]], [b[2], b[3]]] {
            buf.push(word[0]);
            buf.push(word[1]);
            buf.push(crc8(&word));
        }
    }

    /// Build the 10-value measurement payload from the current concentrations.
    fn measured_values(&mut self) -> Vec<u8> {
        let pm1_0 = self.pm1_0.max(0.0);
        let pm2_5 = self.pm2_5.max(0.0);
        let pm4_0 = self.pm4_0.max(0.0);
        let pm10 = self.pm10.max(0.0);
        // Number concentrations (#/cm³) scale roughly with PM2.5 mass; the
        // SPS30's optical count model is not simulated.
        let n0_5 = pm2_5 * 3.5;
        let n1_0 = pm2_5 * 4.0;
        let n2_5 = pm2_5 * 4.1;
        let n4_0 = pm2_5 * 4.15;
        let n10 = pm2_5 * 4.2;
        let typ_size = 0.55_f64; // µm, typical

        let values = [
            pm1_0, pm2_5, pm4_0, pm10, n0_5, n1_0, n2_5, n4_0, n10, typ_size,
        ];

        match self.mode {
            Mode::Float => {
                let mut buf = Vec::with_capacity(60);
                for v in values {
                    Self::push_float(&mut buf, v as f32);
                }
                buf
            }
            Mode::Uint16 => {
                // uint16 mode: datasheet sends integers directly (µg/m³, #/cm³,
                // nm for size). Size in nm to keep precision.
                let words: Vec<u16> = values
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| {
                        if i == 9 {
                            (v * 1000.0).round().clamp(0.0, 65535.0) as u16 // µm → nm
                        } else {
                            v.round().clamp(0.0, 65535.0) as u16
                        }
                    })
                    .collect();
                encode_words(&words)
            }
        }
    }

    fn dispatch(&mut self, cmd: u16) {
        self.read_buf.clear();
        self.read_idx = 0;
        match cmd {
            CMD_START_MEASUREMENT => {
                self.running = true;
                // Mode is decided by the param word that follows; default float.
                self.mode = Mode::Float;
            }
            CMD_STOP_MEASUREMENT => self.running = false,
            CMD_READ_DATA_READY => {
                // Low byte 0x01 ⇒ a new measurement is available; the real part
                // reports not-ready (0x0000) until measurement mode is running.
                let flag = if self.running { 0x0001 } else { 0x0000 };
                self.read_buf = encode_words(&[flag]);
            }
            CMD_READ_MEASURED_VALUES => {
                self.read_buf = self.measured_values();
            }
            CMD_GET_SERIAL => {
                // 16 words of ASCII-ish serial; first spells "LEO".
                let mut words = vec![0x4C45, 0x4F30, 0x3100];
                words.resize(16, 0x0000);
                self.read_buf = encode_words(&words);
            }
            CMD_READ_PRODUCT_TYPE => {
                // 4 words = "00080000" ASCII product type (what the driver reads).
                self.read_buf = encode_words(&[0x3030, 0x3038, 0x3030, 0x3030]);
            }
            CMD_READ_VERSION => {
                // 1 word: firmware major.minor = 2.2.
                self.read_buf = encode_words(&[0x0202]);
            }
            CMD_READ_STATUS_REGISTER => {
                // 2 words: device status register = 0 (no fan/laser/speed faults).
                self.read_buf = encode_words(&[0x0000, 0x0000]);
            }
            CMD_READ_AUTO_CLEAN_INTERVAL => {
                // 2 words: 604800 s (1 week, the SPS30 default).
                self.read_buf = encode_words(&[0x0009, 0x3A80]);
            }
            _ => {}
        }
    }
}

impl I2cDevice for Sps30 {
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
        match self.write_buf.len() {
            2 => {
                let cmd = ((self.write_buf[0] as u16) << 8) | (self.write_buf[1] as u16);
                self.dispatch(cmd);
            }
            // start_measurement carries a mode param word at bytes [2..4].
            4 => {
                let cmd = ((self.write_buf[0] as u16) << 8) | (self.write_buf[1] as u16);
                if cmd == CMD_START_MEASUREMENT {
                    self.mode = if self.write_buf[2] == 0x05 {
                        Mode::Uint16
                    } else {
                        Mode::Float
                    };
                }
            }
            _ => {}
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

/// Drivable channels: the four mass concentrations the SPS30 actually reports,
/// in µg/m³. The 1000 µg/m³ ceiling is the datasheet's specified mass range.
/// ONE table backs BOTH the `SimInput` impl and the kit metadata.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "pm1_0",
        label: "PM1.0",
        unit: "µg/m³",
        min: 0.0,
        max: 1000.0,
    },
    crate::sim_input::InputChannel {
        key: "pm2_5",
        label: "PM2.5",
        unit: "µg/m³",
        min: 0.0,
        max: 1000.0,
    },
    crate::sim_input::InputChannel {
        key: "pm4_0",
        label: "PM4.0",
        unit: "µg/m³",
        min: 0.0,
        max: 1000.0,
    },
    crate::sim_input::InputChannel {
        key: "pm10",
        label: "PM10",
        unit: "µg/m³",
        min: 0.0,
        max: 1000.0,
    },
];

impl crate::sim_input::SimInput for Sps30 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "pm1_0" => self.pm1_0 = value,
            "pm2_5" => self.pm2_5 = value,
            "pm4_0" => self.pm4_0 = value,
            "pm10" => self.pm10 = value,
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

pub struct Sps30Kit;
pub static SPS30_KIT: Sps30Kit = Sps30Kit;

static SPS30_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "sps30",
    label: "Sensirion SPS30 PM",
    summary: "Laser particulate-matter sensor (PM1/2.5/4/10) over I2C.",
    detail: "Sensirion SPS30 at fixed address 0x69, speaking the real Sensirion \
             command protocol with CRC-8 (poly 0x31) in both float and uint16 output \
             modes. The four mass concentrations are externally driven inputs \
             (channels pm1_0 / pm2_5 / pm4_0 / pm10); config seeds them from one \
             pm2_5 value. Drive it in uint16 mode on the ESP32-C3 (30-byte frame \
             fits the controller's 32-byte FIFO).",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to the SPS30 fixed address 0x69.",
        },
        ConfigKey {
            name: "pm2_5",
            ty: ConfigType::Float,
            doc: "Initial PM2.5 mass, µg/m³. Default 6.0 (clean indoor air). The other \
                  mass bins are seeded from it with the typical urban-aerosol ratios; \
                  each bin is drivable at runtime via its own input channel.",
        },
    ],
    labs: &[LabRef {
        board_id: "esp32c3-leo-airquality",
        chip: "esp32c3",
        example_dir: "esp32c3-leo-airquality",
        demo_elf: "demo-esp32c3-leo-airquality.elf",
    }],
};

impl PeripheralKit for Sps30Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &SPS30_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(SPS30_ADDR)?;
        let pm2_5 = ctx.config_f64("pm2_5").unwrap_or(6.0);
        ctx.attach_i2c_device(Box::new(Sps30::new(address, pm2_5)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(d: &mut Sps30, bytes: &[u8]) {
        d.start();
        for &b in bytes {
            d.write(b);
        }
    }

    fn read_n(d: &mut Sps30, n: usize) -> Vec<u8> {
        d.start();
        (0..n).map(|_| d.read()).collect()
    }

    /// Decode an SPS30 float value (2 words + CRCs) at byte offset `i*6`.
    fn decode_float(buf: &[u8], idx: usize) -> f32 {
        let o = idx * 6;
        let bytes = [buf[o], buf[o + 1], buf[o + 3], buf[o + 4]];
        f32::from_be_bytes(bytes)
    }

    #[test]
    fn address_defaults_to_0x69() {
        assert_eq!(Sps30::new_default(0).address(), 0x69);
    }

    #[test]
    fn float_mode_returns_60_bytes_with_valid_crcs() {
        let mut d = Sps30::new_default(SPS30_ADDR);
        send(&mut d, &[0x00, 0x10, 0x03, 0x00, crc8(&[0x03, 0x00])]); // start float
        send(&mut d, &[0x03, 0x00]); // read_measured_values
        let b = read_n(&mut d, 60);
        assert_eq!(b.len(), 60);
        for chunk in b.chunks(3) {
            assert_eq!(chunk[2], crc8(&chunk[..2]), "valid CRC per word");
        }
    }

    #[test]
    fn pm2_5_is_second_value_and_holds_until_driven() {
        use crate::sim_input::SimInput;
        let mut d = Sps30::new_default(SPS30_ADDR);
        send(&mut d, &[0x00, 0x10, 0x03, 0x00, crc8(&[0x03, 0x00])]);
        let mut read_pm = |d: &mut Sps30| {
            send(d, &[0x03, 0x00]);
            let b = read_n(d, 60);
            (
                decode_float(&b, 0),
                decode_float(&b, 1),
                decode_float(&b, 3),
            )
        };
        for _ in 0..20 {
            let (_, pm2_5, _) = read_pm(&mut d);
            assert!((pm2_5 - 6.0).abs() < 1e-4, "no self-running scene: {pm2_5}");
        }
        d.set_input("pm2_5", 22.0).unwrap();
        let (pm1_0, pm2_5, pm10) = read_pm(&mut d);
        assert!((pm2_5 - 22.0).abs() < 1e-3, "driven PM2.5: {pm2_5}");
        // The other bins are independent channels: driving PM2.5 alone leaves
        // them where they were seeded.
        assert!(
            (pm1_0 - 6.0 * 0.78).abs() < 1e-3,
            "pm1_0 unchanged: {pm1_0}"
        );
        d.set_input("pm10", 30.0).unwrap();
        let (_, _, pm10_after) = read_pm(&mut d);
        assert!((pm10 - 6.0 * 1.18).abs() < 1e-3);
        assert!(
            (pm10_after - 30.0).abs() < 1e-3,
            "driven PM10: {pm10_after}"
        );
    }

    #[test]
    fn out_of_range_input_is_rejected() {
        use crate::sim_input::SimInput;
        let mut d = Sps30::new_default(SPS30_ADDR);
        assert!(d.set_input("pm2_5", 2000.0).is_err());
        assert!(d.set_input("pm0_3", 1.0).is_err());
    }

    #[test]
    fn data_ready_flag_gated_on_running() {
        let mut d = Sps30::new_default(SPS30_ADDR);
        // Idle (no start_measurement): the real part reports not-ready.
        send(&mut d, &[0x02, 0x02]);
        let idle = read_n(&mut d, 3);
        assert_eq!(idle[1], 0x00, "not-ready until measurement starts");
        assert_eq!(idle[2], crc8(&idle[..2]));
        // After start_measurement: ready.
        send(&mut d, &[0x00, 0x10, 0x05, 0x00, crc8(&[0x05, 0x00])]);
        send(&mut d, &[0x02, 0x02]);
        let running = read_n(&mut d, 3);
        assert_eq!(running[1], 0x01, "ready once measuring");
        assert_eq!(running[2], crc8(&running[..2]));
    }

    #[test]
    fn identity_commands_return_valid_crc_words() {
        let mut d = Sps30::new_default(SPS30_ADDR);
        for cmd in [[0xD0, 0x02], [0xD1, 0x00], [0xD2, 0x06], [0x80, 0x04]] {
            send(&mut d, &cmd);
            let b = read_n(&mut d, 3);
            assert_ne!(
                b[0], 0xFF,
                "identity cmd {cmd:02X?} must return a real word"
            );
            assert_eq!(b[2], crc8(&b[..2]), "valid CRC for {cmd:02X?}");
        }
    }

    #[test]
    fn uint16_mode_returns_30_bytes() {
        let mut d = Sps30::new_default(SPS30_ADDR);
        send(&mut d, &[0x00, 0x10, 0x05, 0x00, crc8(&[0x05, 0x00])]); // start uint16
        send(&mut d, &[0x03, 0x00]);
        let b = read_n(&mut d, 30);
        assert_eq!(b.len(), 30);
        for chunk in b.chunks(3) {
            assert_eq!(chunk[2], crc8(&chunk[..2]));
        }
    }
}
