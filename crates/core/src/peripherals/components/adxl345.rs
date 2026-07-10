// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;

/// ADXL345 3-axis accelerometer I2C component.
#[derive(Debug, serde::Serialize)]
pub struct Adxl345 {
    address: u8,
    current_register: u8,
    register_address_written: bool,
    power_ctl: u8,
    data_format: u8,
    bw_rate: u8,
    sample_x: i16,
    sample_y: i16,
    sample_z: i16,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Adxl345 {
    fn default() -> Self {
        Self::new(0x53)
    }
}

impl Adxl345 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            power_ctl: 0,
            data_format: 0,
            bw_rate: 0x0A,
            sample_x: 0,
            sample_y: 0,
            sample_z: 256,
            component_id: None,
        }
    }

    pub fn set_sample(&mut self, x: i16, y: i16, z: i16) {
        self.sample_x = x;
        self.sample_y = y;
        self.sample_z = z;
    }

    pub fn sample(&self) -> (i16, i16, i16) {
        (self.sample_x, self.sample_y, self.sample_z)
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            0x00 => 0xE5,
            0x2C => self.bw_rate,
            0x2D => self.power_ctl,
            0x31 => self.data_format,
            0x32 => self.sample_x as u16 as u8,
            0x33 => ((self.sample_x as u16) >> 8) as u8,
            0x34 => self.sample_y as u16 as u8,
            0x35 => ((self.sample_y as u16) >> 8) as u8,
            0x36 => self.sample_z as u16 as u8,
            0x37 => ((self.sample_z as u16) >> 8) as u8,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0x2C => self.bw_rate = value,
            0x2D => self.power_ctl = value,
            0x31 => self.data_format = value,
            _ => {}
        }
    }
}

impl I2cDevice for Adxl345 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let value = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        value
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

/// Drivable accelerometer axes, in g, physical full-scale ±16 g. The
/// conversion follows the LIVE `data_format` register the firmware wrote:
/// full-res mode (bit 3) is always 3.9 mg/LSB (256 counts = 1 g — the
/// model's default rest sample is z = 256); fixed 10-bit mode halves the
/// counts-per-g per range step (±2g→256, ±4g→128, ±8g→64, ±16g→32). Values
/// beyond the configured range saturate, like the silicon. One table backs
/// BOTH the `SimInput` impl and the kit metadata, so the device schema and
/// the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "x",
        label: "X",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "y",
        label: "Y",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "z",
        label: "Z",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
];

impl crate::sim_input::SimInput for Adxl345 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let range_bits = (self.data_format & 0x03) as u32; // 0=±2g … 3=±16g
        let full_res = self.data_format & 0x08 != 0;
        let counts_per_g = if full_res {
            256.0
        } else {
            (256 >> range_bits) as f64
        };
        let full_scale = (2 << range_bits) as f64;
        let raw = (value.clamp(-full_scale, full_scale) * counts_per_g).round() as i16;
        match key {
            "x" => self.sample_x = raw,
            "y" => self.sample_y = raw,
            "z" => self.sample_z = raw,
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

pub struct Adxl345Kit;
pub static ADXL345_KIT: Adxl345Kit = Adxl345Kit;

static ADXL345_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "adxl345",
    label: "ADXL345 Tilt",
    summary: "3-axis ±2/4/8/16 g digital accelerometer over I2C.",
    detail: "Analog Devices ADXL345 with the canonical 0x53 / 0x1D 7-bit address pair. \
             Wired through the simulated I2C bus; host stimulus seeds X/Y/Z and the firmware \
             reads them through the DATAX/Y/Z registers.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x53; 0x1D selects the alternate-pin variant.",
    }],
    labs: &[LabRef {
        board_id: "adxl345-sensor-lab",
        chip: "stm32f103",
        example_dir: "adxl345-sensor-lab",
        demo_elf: "demo-adxl345-sensor-lab.elf",
    }],
};

impl PeripheralKit for Adxl345Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &ADXL345_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x53)?;
        ctx.attach_i2c_device(Box::new(Adxl345::new(address)))?;
        Ok(())
    }
}
