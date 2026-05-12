// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory: build an [`I2cDevice`] from a system-manifest `external_devices`
//! entry's `type:` string + `config:` map.
//!
//! Called by the bus loader (`crates/core/src/bus/mod.rs`) for every
//! `external_devices` entry whose `connection:` id matches an i2c
//! peripheral declared in the chip yaml. Unknown types return `None`
//! so a yaml typo doesn't silently produce an empty bus.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::peripherals::i2c::I2cDevice;

pub fn build_i2c_device(
    type_str: &str,
    config: &HashMap<String, serde_yaml::Value>,
) -> Option<Box<dyn I2cDevice>> {
    match type_str.to_ascii_lowercase().as_str() {
        "tmp102" => Some(Box::new(crate::peripherals::esp32s3::tmp102::Tmp102::new())),
        "mpu6050" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x68) as u8;
            Some(Box::new(crate::peripherals::components::Mpu6050::new(
                address,
            )))
        }
        "aht20" => Some(Box::new(crate::peripherals::components::Aht20::new())),
        "bmp280" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x76) as u8;
            Some(Box::new(crate::peripherals::components::Bmp280::new(
                address,
            )))
        }
        "shm_imu" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x24) as u8;
            let shm_path = config
                .get("shm_path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp/labwired_proximity_imu"));
            let size = config
                .get("size")
                .and_then(|v| v.as_u64())
                .unwrap_or(128) as usize;
            Some(Box::new(crate::peripherals::components::ShmImu::new(
                address, shm_path, size,
            )))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_type_returns_none() {
        let cfg = HashMap::new();
        assert!(build_i2c_device("definitely_not_a_device", &cfg).is_none());
    }

    #[test]
    fn tmp102_built_at_default_address() {
        let cfg = HashMap::new();
        let dev = build_i2c_device("tmp102", &cfg).expect("tmp102 should build");
        assert_eq!(dev.address(), 0x48);
    }

    #[test]
    fn mpu6050_address_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x69)),
        );
        let dev = build_i2c_device("mpu6050", &cfg).expect("mpu6050 should build");
        assert_eq!(dev.address(), 0x69);
    }

    #[test]
    fn type_string_is_case_insensitive() {
        let cfg = HashMap::new();
        assert!(build_i2c_device("TMP102", &cfg).is_some());
        assert!(build_i2c_device("Tmp102", &cfg).is_some());
    }

    #[test]
    fn shm_imu_built_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x24)),
        );
        cfg.insert(
            "shm_path".to_string(),
            serde_yaml::Value::String("/tmp/labwired_proximity_imu".to_string()),
        );
        let dev = build_i2c_device("shm_imu", &cfg).expect("shm_imu should build");
        assert_eq!(dev.address(), 0x24);
    }
}
