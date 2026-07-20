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

/// [`build_i2c_device`] + identity: builds the device for a system.yaml
/// `external_devices` entry and stamps its id onto the model (when it is an
/// input device) so discovery and the stimulus resolver can address it by the
/// name the author wrote (see [`crate::sim_input::SimInput::component_id`]).
/// Every from-config attach path should use THIS, not the raw builder.
pub fn build_external_i2c_device(
    type_str: &str,
    id: &str,
    config: &HashMap<String, serde_yaml::Value>,
) -> Option<Box<dyn I2cDevice>> {
    let mut dev = build_i2c_device(type_str, config)?;
    if let Some(si) = dev.as_sim_input_mut() {
        si.set_component_id(id.to_string());
    }
    Some(dev)
}

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
        "fxos8700" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x1f) as u8;
            Some(Box::new(crate::peripherals::components::Fxos8700::new(
                address,
            )))
        }
        "aht20" => Some(Box::new(crate::peripherals::components::Aht20::new())),
        // ── Smart-ring sensor/actuator set ──────────────────────────────────
        "max30102" => {
            use crate::peripherals::components::max30102::{Max30102, MAX30102_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(MAX30102_ADDR as u64) as u8;
            let mut dev = Max30102::new(address);
            if let Some(seed) = config.get("seed").and_then(|v| v.as_u64()) {
                dev = dev.with_seed(seed as u32);
            }
            if let Some(bpm) = config.get("heart_rate_bpm").and_then(|v| v.as_f64()) {
                dev = dev.with_heart_rate_bpm(bpm);
            }
            if let Some(on) = config.get("transaction_advance").and_then(|v| v.as_bool()) {
                dev.set_transaction_advance(on);
            }
            Some(Box::new(dev))
        }
        "cap1188" => {
            use crate::peripherals::components::cap1188::{Cap1188, CAP1188_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(CAP1188_ADDR as u64) as u8;
            Some(Box::new(Cap1188::new(address)))
        }
        "drv2605" | "drv2605l" => {
            use crate::peripherals::components::drv2605::{Drv2605, DRV2605_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(DRV2605_ADDR as u64) as u8;
            Some(Box::new(Drv2605::new(address)))
        }
        // scd41 / sgp41 / sps30 / veml7700 are onboarded through the
        // PeripheralKit registry (peripherals/kit), which dispatches them on
        // both the STM32 and ESP32-C3 I²C buses — no legacy arm needed here.
        "bme280" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x76) as u8;
            Some(Box::new(crate::peripherals::components::Bme280::new(
                address,
            )))
        }
        "bmp280" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x76) as u8;
            Some(Box::new(crate::peripherals::components::Bmp280::new(
                address,
            )))
        }
        "mlx90640" => {
            use crate::peripherals::components::mlx90640::{Mlx90640, ThermalScene, MLX90640_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(MLX90640_ADDR as u64) as u8;

            let f = |key: &str, default: f64| -> f64 {
                config.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
            };
            let u = |key: &str, default: u64| -> usize {
                config.get(key).and_then(|v| v.as_u64()).unwrap_or(default) as usize
            };

            let scene = ThermalScene::from_config(
                f("ambient_c", 25.0),
                u("hot_row", 12),
                u("hot_col", 16),
                u("hot_radius", 0),
                f("hot_target_c", 60.0),
                f("load", 1.0),
                f("tau_s", 0.0),
                f("cooling_efficiency", 0.0),
                config.get("cooling_fault_at_s").and_then(|v| v.as_f64()),
                f("frame_period_s", 0.5),
            );
            Some(Box::new(Mlx90640::new(address, scene)))
        }
        "shm_i2c" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x24) as u8;
            let shm_path = config
                .get("shm_path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp/labwired_proximity_imu"));
            let size = config.get("size").and_then(|v| v.as_u64()).unwrap_or(128) as usize;
            Some(Box::new(crate::peripherals::components::ShmI2c::new(
                address, shm_path, size,
            )))
        }
        "ir" => {
            let spec_path = match config.get("spec_path").and_then(|v| v.as_str()) {
                Some(p) => p,
                None => {
                    eprintln!("ir component: missing required 'spec_path' in config");
                    return None;
                }
            };
            let yaml = match std::fs::read_to_string(spec_path) {
                Ok(y) => y,
                Err(e) => {
                    eprintln!("ir component: cannot read {spec_path}: {e}");
                    return None;
                }
            };
            let spec: labwired_ir::component::IrComponent = match serde_yaml::from_str(&yaml) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ir component: {spec_path} parse error: {e}");
                    return None;
                }
            };
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .map(|a| a as u8);
            match crate::peripherals::components::IrI2cComponent::new(spec, address) {
                Ok(d) => Some(Box::new(d)),
                Err(e) => {
                    eprintln!("ir component: {spec_path} invalid: {e}");
                    None
                }
            }
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
    fn smart_ring_devices_build_at_their_default_addresses() {
        let cfg = HashMap::new();
        assert_eq!(build_i2c_device("max30102", &cfg).unwrap().address(), 0x57);
        assert_eq!(build_i2c_device("cap1188", &cfg).unwrap().address(), 0x29);
        assert_eq!(build_i2c_device("drv2605", &cfg).unwrap().address(), 0x5A);
        assert_eq!(build_i2c_device("drv2605l", &cfg).unwrap().address(), 0x5A);
    }

    #[test]
    fn smart_ring_addresses_override_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x2A)),
        );
        assert_eq!(build_i2c_device("cap1188", &cfg).unwrap().address(), 0x2A);
    }

    #[test]
    fn max30102_config_keys_reach_the_model() {
        use crate::peripherals::components::Max30102;
        let mut cfg = HashMap::new();
        cfg.insert(
            "heart_rate_bpm".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(95.0)),
        );
        cfg.insert(
            "seed".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(42)),
        );
        let dev = build_i2c_device("max30102", &cfg).expect("max30102 should build");
        let ppg = dev
            .as_any()
            .and_then(|a| a.downcast_ref::<Max30102>())
            .expect("model is a Max30102");
        assert_eq!(ppg.heart_rate_bpm(), 95.0);
    }

    #[test]
    fn external_attach_stamps_the_component_id_on_input_devices() {
        let cfg = HashMap::new();
        let mut dev = build_external_i2c_device("max30102", "ppg", &cfg).expect("builds");
        let si = dev.as_sim_input_mut().expect("max30102 is a SimInput");
        assert_eq!(si.component_id(), Some("ppg"));

        let mut touch = build_external_i2c_device("cap1188", "touchpad", &cfg).expect("builds");
        let si = touch.as_sim_input_mut().expect("cap1188 is a SimInput");
        assert_eq!(si.component_id(), Some("touchpad"));
    }

    #[test]
    fn type_string_is_case_insensitive() {
        let cfg = HashMap::new();
        assert!(build_i2c_device("TMP102", &cfg).is_some());
        assert!(build_i2c_device("Tmp102", &cfg).is_some());
    }

    #[test]
    fn shm_i2c_built_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x24)),
        );
        cfg.insert(
            "shm_path".to_string(),
            serde_yaml::Value::String("/tmp/labwired_proximity_imu".to_string()),
        );
        let dev = build_i2c_device("shm_i2c", &cfg).expect("shm_imu should build");
        assert_eq!(dev.address(), 0x24);
    }

    #[test]
    fn ir_type_builds_from_spec_path() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../configs/components/pca9685.yaml"
                )
                .to_string(),
            ),
        );
        let dev = build_i2c_device("ir", &cfg).expect("ir device should build");
        assert_eq!(dev.address(), 0x40);
    }

    #[test]
    fn ir_type_address_override_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../configs/components/pca9685.yaml"
                )
                .to_string(),
            ),
        );
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x41)),
        );
        let dev = build_i2c_device("ir", &cfg).expect("ir device should build");
        assert_eq!(dev.address(), 0x41);
    }

    #[test]
    fn mlx90640_built_at_default_address() {
        let cfg = HashMap::new();
        let dev = build_i2c_device("mlx90640", &cfg).expect("mlx90640 should build");
        assert_eq!(dev.address(), 0x33);
    }

    #[test]
    fn mlx90640_address_and_scene_from_config() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x33)),
        );
        cfg.insert(
            "ambient_c".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(30.0)),
        );
        cfg.insert(
            "hot_target_c".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(90.0)),
        );
        let dev = build_i2c_device("mlx90640", &cfg).expect("mlx90640 should build");
        assert_eq!(dev.address(), 0x33);
    }

    #[test]
    fn ir_type_missing_or_bad_spec_returns_none() {
        // Missing spec_path.
        assert!(build_i2c_device("ir", &HashMap::new()).is_none());
        // Nonexistent file.
        let mut cfg = HashMap::new();
        cfg.insert(
            "spec_path".to_string(),
            serde_yaml::Value::String("/nonexistent/spec.yaml".to_string()),
        );
        assert!(build_i2c_device("ir", &cfg).is_none());
    }
}
