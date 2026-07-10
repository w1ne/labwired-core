// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::i2c::I2cDevice;

/// MPU6050 6-DoF I2C IMU Component
#[derive(Debug, serde::Serialize)]
pub struct Mpu6050 {
    address: u8,
    current_register: u8,

    // Core registers
    pwr_mgmt_1: u8,
    who_am_i: u8,

    // Sensor data (dummy static values for now, but could be dynamic)
    accel_x: i16,
    accel_y: i16,
    accel_z: i16,
    gyro_x: i16,
    gyro_y: i16,
    gyro_z: i16,

    // Full-scale config registers, latched so the input conversion follows
    // what the firmware actually configured (not an assumed power-on scale).
    accel_config: u8,
    gyro_config: u8,

    // Internal state tracking for I2C register pointer
    register_address_written: bool,

    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl Default for Mpu6050 {
    fn default() -> Self {
        Self::new(0x68) // Default I2C address for MPU6050
    }
}

impl Mpu6050 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            current_register: 0,
            pwr_mgmt_1: 0x40, // Reset value (sleep mode bit set)
            who_am_i: 0x68,

            // Dummy calibration/data
            accel_x: 0x0123,
            accel_y: 0x0456,
            accel_z: 0x4000, // Roughly 1g depending on scale
            gyro_x: 0x0010,
            gyro_y: 0x0020,
            gyro_z: 0x0030,

            accel_config: 0,
            gyro_config: 0,
            register_address_written: false,
            component_id: None,
        }
    }

    fn read_register(&self, reg: u8) -> u8 {
        match reg {
            0x3B => (self.accel_x >> 8) as u8,
            0x3C => (self.accel_x & 0xFF) as u8,
            0x3D => (self.accel_y >> 8) as u8,
            0x3E => (self.accel_y & 0xFF) as u8,
            0x3F => (self.accel_z >> 8) as u8,
            0x40 => (self.accel_z & 0xFF) as u8,

            0x43 => (self.gyro_x >> 8) as u8,
            0x44 => (self.gyro_x & 0xFF) as u8,
            0x45 => (self.gyro_y >> 8) as u8,
            0x46 => (self.gyro_y & 0xFF) as u8,
            0x47 => (self.gyro_z >> 8) as u8,
            0x48 => (self.gyro_z & 0xFF) as u8,

            0x1B => self.gyro_config,
            0x1C => self.accel_config,

            0x6B => self.pwr_mgmt_1,
            0x75 => self.who_am_i,
            _ => 0,
        }
    }

    fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0x1B => self.gyro_config = value,
            0x1C => self.accel_config = value,
            0x6B => self.pwr_mgmt_1 = value,
            _ => {}
        }
    }

    pub fn set_sample(&mut self, ax: i16, ay: i16, az: i16, gx: i16, gy: i16, gz: i16) {
        self.accel_x = ax;
        self.accel_y = ay;
        self.accel_z = az;
        self.gyro_x = gx;
        self.gyro_y = gy;
        self.gyro_z = gz;
    }

    pub fn sample(&self) -> (i16, i16, i16, i16, i16, i16) {
        (
            self.accel_x,
            self.accel_y,
            self.accel_z,
            self.gyro_x,
            self.gyro_y,
            self.gyro_z,
        )
    }

    pub fn simulate_motion(&mut self) {
        // Simple tick function to alter data slightly if needed
        self.accel_x = self.accel_x.wrapping_add(10);
        self.gyro_z = self.gyro_z.wrapping_sub(5);
    }
}

impl I2cDevice for Mpu6050 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        // Read current register and auto-increment
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

/// Drivable 6-DoF channels. The conversion follows the LIVE full-scale the
/// firmware configured (ACCEL_CONFIG AFS_SEL: ±2/4/8/16 g at 16384/8192/
/// 4096/2048 LSB/g; GYRO_CONFIG FS_SEL: ±250/500/1000/2000 °/s at 131/65.5/
/// 32.8/16.4 LSB/(°/s)). The schema range below is the hardware maximum;
/// values beyond the configured full-scale saturate, like the silicon. One
/// table backs BOTH the `SimInput` impl and the kit metadata, so the device
/// schema and the runtime API cannot drift.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "ax",
        label: "Accel X",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "ay",
        label: "Accel Y",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "az",
        label: "Accel Z",
        unit: "g",
        min: -16.0,
        max: 16.0,
    },
    crate::sim_input::InputChannel {
        key: "gx",
        label: "Gyro X",
        unit: "°/s",
        min: -2000.0,
        max: 2000.0,
    },
    crate::sim_input::InputChannel {
        key: "gy",
        label: "Gyro Y",
        unit: "°/s",
        min: -2000.0,
        max: 2000.0,
    },
    crate::sim_input::InputChannel {
        key: "gz",
        label: "Gyro Z",
        unit: "°/s",
        min: -2000.0,
        max: 2000.0,
    },
];

impl crate::sim_input::SimInput for Mpu6050 {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let afs = ((self.accel_config >> 3) & 0x03) as u32;
        let accel_fs = (2 << afs) as f64; // ±2/4/8/16 g
        let accel_lsb_per_g = (16384 >> afs) as f64;
        let fs = ((self.gyro_config >> 3) & 0x03) as u32;
        let gyro_fs = (250 << fs) as f64; // ±250/500/1000/2000 °/s
        let gyro_lsb_per_dps = 131.0 / (1 << fs) as f64;
        let accel = |v: f64| (v.clamp(-accel_fs, accel_fs) * accel_lsb_per_g).round() as i16;
        let gyro = |v: f64| (v.clamp(-gyro_fs, gyro_fs) * gyro_lsb_per_dps).round() as i16;
        match key {
            "ax" => self.accel_x = accel(value),
            "ay" => self.accel_y = accel(value),
            "az" => self.accel_z = accel(value),
            "gx" => self.gyro_x = gyro(value),
            "gy" => self.gyro_y = gyro(value),
            "gz" => self.gyro_z = gyro(value),
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

pub struct Mpu6050Kit;
pub static MPU6050_KIT: Mpu6050Kit = Mpu6050Kit;

static MPU6050_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "mpu6050",
    label: "MPU6050 IMU",
    summary: "6-axis gyro + accelerometer over I2C.",
    detail: "InvenSense MPU-6050 with WHO_AM_I = 0x68. Reports static sample values today; \
             host stimulus hooks into the WASM bridge for live updates.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x68; 0x69 selects the AD0=high variant.",
    }],
    labs: &[LabRef {
        board_id: "mpu6050-sensor-lab",
        chip: "stm32f103",
        example_dir: "mpu6050-sensor-lab",
        demo_elf: "demo-mpu6050-sensor-lab.elf",
    }],
};

impl PeripheralKit for Mpu6050Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MPU6050_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x68)?;
        ctx.attach_i2c_device(Box::new(Mpu6050::new(address)))?;
        Ok(())
    }
}
