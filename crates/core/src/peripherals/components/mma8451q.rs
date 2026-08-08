// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP MMA8451Q 3-axis accelerometer (I²C, datasheet MMA8451Q rev 11).
//! Register-pointer protocol identical in shape to the MPU6050 model.
//! Fidelity: standby/active gating (CTRL_REG1.ACTIVE) — output registers read
//! zero until the part is active, matching a part that has not converted yet;
//! 14-bit left-justified output; full-scale select via XYZ_DATA_CFG.
//! Intentionally out of scope (same as Simulator86): tap/orientation/freefall
//! detection, FIFO, INT1/INT2 assertion. Standby reads return 0 rather than
//! the last pre-standby conversion — documented limitation. The real part
//! NACKs while unpowered; the bus trait has no NACK path.

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::noise::ChannelNoise;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

const WHO_AM_I: u8 = 0x1A;

#[derive(Debug, serde::Serialize)]
pub struct Mma8451q {
    address: u8,
    current_register: u8,
    register_address_written: bool,

    ctrl_reg1: u8,
    xyz_data_cfg: u8,

    /// Latest driven engineering values per axis (g), latched on `set_input`
    /// regardless of power state — the value is there the moment firmware
    /// wakes the part. Converted to counts at read time while ACTIVE.
    pending: [f64; 3],

    component_id: Option<String>,
    #[serde(skip)]
    noise_sigma: f64,
    #[serde(skip)]
    noise: Option<[ChannelNoise; 3]>,
}

pub const INPUT_CHANNELS: &[InputChannel] = &[
    InputChannel {
        key: "x",
        label: "Accel X",
        unit: "g",
        min: -8.0,
        max: 8.0,
    },
    InputChannel {
        key: "y",
        label: "Accel Y",
        unit: "g",
        min: -8.0,
        max: 8.0,
    },
    InputChannel {
        key: "z",
        label: "Accel Z",
        unit: "g",
        min: -8.0,
        max: 8.0,
    },
];

impl Mma8451q {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            register_address_written: false,
            ctrl_reg1: 0,
            xyz_data_cfg: 0,
            pending: [0.0; 3],
            component_id: None,
            noise_sigma: 0.0,
            noise: None,
        }
    }

    /// Enable seeded per-axis Gaussian noise (sigma in g). The states are
    /// (re)keyed when the component id is stamped at attach.
    pub fn with_noise_sigma(mut self, sigma: f64) -> Self {
        self.noise_sigma = sigma;
        self
    }

    fn active(&self) -> bool {
        self.ctrl_reg1 & 0x01 != 0
    }

    /// Counts per g for the configured full scale (14-bit output).
    fn counts_per_g(&self) -> f64 {
        match self.xyz_data_cfg & 0x03 {
            0 => 4096.0, // ±2g
            1 => 2048.0, // ±4g
            _ => 1024.0, // ±8g
        }
    }

    fn rebuild_noise(&mut self) {
        if self.noise_sigma <= 0.0 {
            self.noise = None;
            return;
        }
        let id = self.component_id.clone().unwrap_or_default();
        self.noise = Some(
            ["x", "y", "z"].map(|ch| ChannelNoise::new(0, &id, ch, self.noise_sigma, 0.0, None)),
        );
    }

    fn read_register(&mut self, reg: u8) -> u8 {
        match reg {
            0x01..=0x06 => {
                // Standby: no conversions, output registers read zero.
                if !self.active() {
                    return 0;
                }
                let axis = ((reg - 0x01) / 2) as usize;
                let cpg = self.counts_per_g();
                let fs_g = 8192.0 / cpg;
                let g = self.pending[axis].clamp(-fs_g, fs_g);
                let g = match self.noise.as_mut() {
                    Some(noise) => noise[axis].sample(g, None),
                    None => g,
                };
                let counts = (g * cpg).round() as i16;
                let justified = (counts << 2) as u16; // 14-bit left-justified
                if reg % 2 == 1 {
                    (justified >> 8) as u8
                } else {
                    (justified & 0xFF) as u8
                }
            }
            0x0D => WHO_AM_I,
            0x0E => self.xyz_data_cfg,
            0x2A => self.ctrl_reg1,
            _ => {
                crate::census_reg!("components.mma8451q:Mma8451q", reg, "read");
                0
            }
        }
    }
}

impl I2cDevice for Mma8451q {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        let val = self.read_register(self.current_register);
        self.current_register = self.current_register.wrapping_add(1);
        val
    }

    fn write(&mut self, data: u8) {
        if !self.register_address_written {
            // First byte written is the register address
            self.current_register = data;
            self.register_address_written = true;
        } else {
            // Subsequent bytes are data
            match self.current_register {
                0x0E => self.xyz_data_cfg = data & 0x03,
                0x2A => self.ctrl_reg1 = data,
                _ => {}
            }
            self.current_register = self.current_register.wrapping_add(1);
        }
    }

    fn stop(&mut self) {
        self.register_address_written = false;
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl SimInput for Mma8451q {
    fn input_channels(&self) -> &'static [InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let axis = match key {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => unreachable!("require_channel validated the key"),
        };
        self.pending[axis] = value;
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
        self.rebuild_noise();
    }
}

// ---- Kit ----

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Mma8451qKit;
pub static MMA8451Q_KIT: Mma8451qKit = Mma8451qKit;

static MMA8451Q_METADATA: KitMetadata = KitMetadata {
    device_type: "mma8451q",
    label: "MMA8451Q Accelerometer",
    summary: "3-axis 14-bit accelerometer over I2C with standby/active gating.",
    detail: "NXP MMA8451Q, WHO_AM_I = 0x1A. XYZ held at zero in standby, converts while \
             CTRL_REG1.ACTIVE; 2/4/8 g full scale. Optional seeded noise via \
             noise_sigma (g). Tap/FIFO/interrupts not modelled.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to 0x1C; 0x1D selects SA0=high.",
        },
        ConfigKey {
            name: "noise_sigma",
            ty: ConfigType::Float,
            doc: "Optional Gaussian noise sigma in g, seeded and replay-safe.",
        },
    ],
    labs: &[],
    inputs: INPUT_CHANNELS,
};

impl PeripheralKit for Mma8451qKit {
    fn metadata(&self) -> &'static KitMetadata {
        &MMA8451Q_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x1C)?;
        let sigma = ctx.config_f64("noise_sigma").unwrap_or(0.0);
        ctx.attach_i2c_device(Box::new(Mma8451q::new(address).with_noise_sigma(sigma)))?;
        Ok(())
    }
}
