// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Melexis **MLX90614** single-point IR thermometer as an [`I2cDevice`].
//!
//! The MLX90614 is the cheap (~$5) non-contact IR thermometer that gives the Leo
//! board the one thing an air-only humidity model can't have: the **temperature
//! of a cold surface** (a window, an exterior wall, a thermal bridge). Mould
//! starts at the cold surface where the *air* relative humidity lies — the
//! surface RH can be at condensation (100 %) while the room air reads a benign
//! 70 %. Reading the surface temperature lets the firmware compute the dew point
//! and flag condensation *before* visible mould, which is the entire premise of
//! the moisture-first "Leo v1" product.
//!
//! ## SMBus protocol (datasheet §8.4.3, "Read word")
//! The master issues an SMBus *read word*: write the 1-byte RAM command, repeated
//! START, then read **LSB, MSB, PEC** (little-endian word + CRC-8). RAM commands:
//! - `0x06` Ta    — ambient (chip) temperature
//! - `0x07` TOBJ1 — object 1 temperature (what the optics point at: the surface)
//! - `0x08` TOBJ2 — object 2 (single-zone parts mirror TOBJ1)
//!
//! Temperature encoding is `T[K] = raw × 0.02`, so `T[°C] = raw × 0.02 − 273.15`
//! and `raw = (T[°C] + 273.15) × 50`. The **PEC** is an SMBus CRC-8 (poly `0x07`,
//! init 0) over `[addr·W, cmd, addr·R, LSB, MSB]`, so a driver that validates the
//! checksum (as good MLX drivers do) sees a correct one.
//!
//! Both temperatures are externally driven variables: they change only when
//! something drives them through the ONE stimulus contract,
//! [`crate::sim_input::SimInput`] (channels `surface_temp`, `ambient_temp`).
//! Config only seeds their initial values. Driving the surface below the air's
//! dew point is what makes the firmware's condensation flag fire.

use crate::peripherals::i2c::I2cDevice;

pub const MLX90614_ADDR: u8 = 0x5A;

const CMD_TA: u8 = 0x06;
const CMD_TOBJ1: u8 = 0x07;
const CMD_TOBJ2: u8 = 0x08;

/// Encode a Celsius temperature into the MLX90614's `raw × 0.02 K` word.
fn celsius_to_raw(t_c: f64) -> u16 {
    (((t_c + 273.15) * 50.0).round()).clamp(0.0, 0x7FFF as f64) as u16
}

/// SMBus PEC: CRC-8, polynomial `x^8+x^2+x+1` (0x07), initial value 0.
fn smbus_pec(bytes: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// MLX90614 model.
pub struct Mlx90614 {
    address: u8,
    /// Surface (object) temperature, °C — the cold spot. Externally driven.
    surface: f64,
    /// Ambient (chip) temperature, °C — roughly the room air. Externally driven.
    ambient: f64,
    /// RAM command selected by the preceding write (0x06 Ta / 0x07 TOBJ1 …).
    pointer: u8,
    /// Bytes written this transaction (the 1-byte command).
    write_buf: Vec<u8>,
    /// Latched 3-byte SMBus response (LSB, MSB, PEC) and the next byte index.
    response: [u8; 3],
    read_byte_idx: usize,
    latched: bool,
    /// system.yaml `external_devices` id, stamped at attach.
    component_id: Option<String>,
}

impl Mlx90614 {
    /// `surface` and `ambient` are the initial temperatures in °C.
    pub fn new(address: u8, surface: f64, ambient: f64) -> Self {
        let address = if address == 0 { MLX90614_ADDR } else { address };
        Self {
            address,
            surface,
            ambient,
            pointer: CMD_TOBJ1,
            write_buf: Vec::with_capacity(2),
            response: [0; 3],
            read_byte_idx: 0,
            latched: false,
            component_id: None,
        }
    }

    /// Plausible winter-evening defaults: a 18 °C window in ~22 °C room air.
    pub fn new_default(address: u8) -> Self {
        Self::new(address, 18.0, 22.0)
    }

    /// Latch the 3-byte response for the selected RAM command.
    fn latch_response(&mut self) {
        let raw = match self.pointer {
            CMD_TOBJ1 | CMD_TOBJ2 => celsius_to_raw(self.surface),
            CMD_TA => celsius_to_raw(self.ambient),
            // Unknown RAM/EEPROM address: report the surface so a probing driver
            // still gets plausible data rather than zero.
            _ => celsius_to_raw(self.surface),
        };
        let lsb = (raw & 0xFF) as u8;
        let msb = (raw >> 8) as u8;
        let addr_w = self.address << 1;
        let addr_r = (self.address << 1) | 1;
        let pec = smbus_pec(&[addr_w, self.pointer, addr_r, lsb, msb]);
        self.response = [lsb, msb, pec];
        self.read_byte_idx = 0;
        self.latched = true;
    }

    /// Current surface temperature, °C (for tests / inspection).
    pub fn surface_temp_c(&self) -> f64 {
        self.surface
    }
}

impl I2cDevice for Mlx90614 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // A (repeated) START frames either the command write or the read phase.
        // Clear the write accumulator and read cursor; keep the pointer set by
        // the preceding command write so the read returns the selected word.
        self.write_buf.clear();
        self.read_byte_idx = 0;
        self.latched = false;
    }

    fn stop(&mut self) {
        // The C3 controller only calls start() on a repeated START, so clear the
        // command accumulator at transaction end too — otherwise a stale command
        // byte would corrupt the next transaction's RAM pointer.
        self.write_buf.clear();
    }

    fn write(&mut self, data: u8) {
        self.write_buf.push(data);
        if self.write_buf.len() == 1 {
            self.pointer = data; // the RAM command byte
        }
    }

    fn read(&mut self) -> u8 {
        if !self.latched {
            self.latch_response();
        }
        let byte = self
            .response
            .get(self.read_byte_idx)
            .copied()
            .unwrap_or(0xFF);
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

/// Drivable channels, in °C. Ranges are the MLX90614's specified measurement
/// spans: -70..380 °C for the object, -40..125 °C for the chip's own ambient.
/// ONE table backs BOTH the `SimInput` impl and the kit metadata.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "surface_temp",
        label: "Surface temperature",
        unit: "°C",
        min: -70.0,
        max: 380.0,
    },
    crate::sim_input::InputChannel {
        key: "ambient_temp",
        label: "Ambient temperature",
        unit: "°C",
        min: -40.0,
        max: 125.0,
    },
];

impl crate::sim_input::SimInput for Mlx90614 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "surface_temp" => self.surface = value,
            "ambient_temp" => self.ambient = value,
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

pub struct Mlx90614Kit;
pub static MLX90614_KIT: Mlx90614Kit = Mlx90614Kit;

static MLX90614_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "mlx90614",
    label: "Melexis MLX90614 IR Temp",
    summary: "Single-point IR (non-contact) thermometer over SMBus/I2C.",
    detail: "Melexis MLX90614 at address 0x5A. Reports an SMBus read-word (LSB, MSB, \
             PEC) for ambient (0x06) and object/surface (0x07) temperature, encoded \
             raw×0.02 K with a correct CRC-8 PEC. Both temperatures are externally \
             driven inputs (channels surface_temp / ambient_temp); driving the surface \
             below the air dew point is what makes firmware flag the surface \
             condensation an air-only humidity index misses.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to the MLX90614 fixed address 0x5A.",
        },
        ConfigKey {
            name: "surface_temp_c",
            ty: ConfigType::Float,
            doc: "Initial surface (object) temperature, °C. Default 18. Drive it at \
                  runtime with the `surface_temp` input channel.",
        },
        ConfigKey {
            name: "ambient_temp_c",
            ty: ConfigType::Float,
            doc: "Initial ambient (chip) temperature reported on Ta, °C. Default 22. \
                  Runtime channel: `ambient_temp`.",
        },
    ],
    labs: &[LabRef {
        board_id: "esp32c3-leo-airquality",
        chip: "esp32c3",
        example_dir: "esp32c3-leo-airquality",
        demo_elf: "demo-esp32c3-leo-airquality.elf",
    }],
};

impl PeripheralKit for Mlx90614Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MLX90614_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(MLX90614_ADDR)?;
        let surface = ctx.config_f64("surface_temp_c").unwrap_or(18.0);
        let ambient = ctx.config_f64("ambient_temp_c").unwrap_or(22.0);
        ctx.attach_i2c_device(Box::new(Mlx90614::new(address, surface, ambient)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue an SMBus read-word for a RAM command, returning (raw, pec).
    fn read_word(d: &mut Mlx90614, cmd: u8) -> (u16, u8) {
        d.start();
        d.write(cmd);
        d.start(); // repeated START into the read phase
        let lsb = d.read() as u16;
        let msb = d.read() as u16;
        let pec = d.read();
        ((msb << 8) | lsb, pec)
    }

    fn raw_to_celsius(raw: u16) -> f64 {
        raw as f64 * 0.02 - 273.15
    }

    #[test]
    fn address_defaults_to_0x5a() {
        assert_eq!(Mlx90614::new_default(0).address(), 0x5A);
    }

    #[test]
    fn object_read_is_plausible_surface_temp() {
        let mut d = Mlx90614::new_default(MLX90614_ADDR);
        let (raw, _) = read_word(&mut d, CMD_TOBJ1);
        let t = raw_to_celsius(raw);
        assert!(
            (8.0..20.0).contains(&t),
            "first surface read near 18 °C: {t:.1}"
        );
    }

    #[test]
    fn ambient_channel_reports_room_temp_without_moving_scene() {
        let mut d = Mlx90614::new_default(MLX90614_ADDR);
        let (ta1, _) = read_word(&mut d, CMD_TA);
        let (ta2, _) = read_word(&mut d, CMD_TA);
        assert_eq!(ta1, ta2, "Ta holds until driven");
        assert!((raw_to_celsius(ta1) - 22.0).abs() < 0.1, "Ta ≈ 22 °C");
    }

    #[test]
    fn surface_holds_until_driven() {
        use crate::sim_input::SimInput;
        let mut d = Mlx90614::new_default(MLX90614_ADDR);
        let first = raw_to_celsius(read_word(&mut d, CMD_TOBJ1).0);
        for _ in 0..20 {
            let t = raw_to_celsius(read_word(&mut d, CMD_TOBJ1).0);
            assert!((t - first).abs() < 0.01, "no self-running scene: {t:.2}");
        }
        d.set_input("surface_temp", 8.0).unwrap();
        let cold = raw_to_celsius(read_word(&mut d, CMD_TOBJ1).0);
        assert!((cold - 8.0).abs() < 0.02, "driven to 8 °C: {cold:.2}");
        d.set_input("ambient_temp", 21.0).unwrap();
        let ta = raw_to_celsius(read_word(&mut d, CMD_TA).0);
        assert!((ta - 21.0).abs() < 0.02, "driven Ta: {ta:.2}");
    }

    #[test]
    fn out_of_range_input_is_rejected() {
        use crate::sim_input::SimInput;
        let mut d = Mlx90614::new_default(MLX90614_ADDR);
        assert!(d.set_input("ambient_temp", 200.0).is_err());
        assert!(d.set_input("object_temp", 20.0).is_err());
    }

    #[test]
    fn pec_is_a_correct_smbus_crc8() {
        let mut d = Mlx90614::new_default(MLX90614_ADDR);
        let (raw, pec) = read_word(&mut d, CMD_TOBJ1);
        let lsb = (raw & 0xFF) as u8;
        let msb = (raw >> 8) as u8;
        let expect = smbus_pec(&[
            MLX90614_ADDR << 1,
            CMD_TOBJ1,
            (MLX90614_ADDR << 1) | 1,
            lsb,
            msb,
        ]);
        assert_eq!(pec, expect, "PEC matches the SMBus CRC-8 over the frame");
    }

    #[test]
    fn known_vector_encodes_celsius() {
        // 36.5 °C → raw = (36.5 + 273.15) * 50 ≈ 15482.4999 (f64) → 15482.
        // Decodes back to 36.49 °C, within the part's 0.02 °C quantisation.
        let d = Mlx90614::new(MLX90614_ADDR, 36.5, 22.0);
        assert_eq!(celsius_to_raw(d.surface_temp_c()), 15482);
        assert!((raw_to_celsius(15482) - 36.5).abs() < 0.02);
    }
}
