// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Factory: **construct** an [`I2cDevice`] from a system-manifest
//! `external_devices` entry's `type:` string + `config:` map.
//!
//! # Role vs PeripheralKit
//!
//! **Attach** (what runs on every MCU) goes through
//! [`crate::bus::external_devices::attach_external_device_universal`] →
//! kit registry / declarative YAML. Kit types must **not** be attached
//! from this factory on the hot path (nRF factories skip kit types).
//!
//! **This factory** still builds `Box<dyn I2cDevice>` for:
//! 1. **Mux trees** ([`build_i2c_tree`]) — children need construct-before-attach
//! 2. **Unit tests** that exercise models without a full system bus
//! 3. **Legacy residual** — `tca9548a` / `shm_i2c` (allowlisted in
//!    `i2c_factory_kit_coverage`)
//!
//! Product types that also have a kit may keep a thin construct arm here so
//! mux children of that type still resolve; the kit is the source of
//! metadata and the universal attach path. Do not add a new product type
//! to this match without a kit (the coverage gate fails).

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

// ── I²C bus topology: devices behind a bus switch ───────────────────────────
//
// `external_devices[].connection` normally names a controller peripheral
// ("i2c1"). It may instead name ANOTHER external device's `id`, which is how a
// slave is placed behind a TCA9548A bus switch — the only way several devices
// with the same fixed address (four VCNL4010s, all 0x13) can share one bus.
//
// The manifest stays flat; the tree is reconstructed here, in ONE place, so
// every attach path (generic from_config, nRF52/nRF54L TWIM factories, the
// ESP32 Xtensa glue) assembles the identical topology instead of each growing
// its own idea of what `connection` means.

/// `type:` strings that build an I²C bus switch. Kept next to the factory arm
/// that builds them so the two cannot drift.
pub fn is_i2c_mux_type(type_str: &str) -> bool {
    matches!(
        type_str.to_ascii_lowercase().as_str(),
        "tca9548a" | "pca9548a" | "tca9548"
    )
}

/// Every external-device id that hangs off ANOTHER external device (i.e. sits
/// behind a bus switch) rather than off a controller peripheral.
///
/// Callers use it to keep such a device out of the generic attach loops: it is
/// wired by [`build_i2c_tree`] as part of its parent switch, and attaching it a
/// second time straight onto the controller would put it on the wrong bus
/// segment — the exact silent mis-wiring this whole path exists to prevent.
pub fn i2c_mux_child_ids(manifest: &labwired_config::SystemManifest) -> Vec<&str> {
    let ids: std::collections::HashSet<&str> = manifest
        .external_devices
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    manifest
        .external_devices
        .iter()
        .filter(|e| ids.contains(e.connection.as_str()))
        .map(|e| e.id.as_str())
        .collect()
}

/// Reject a mux topology that cannot be built, BEFORE any peripheral is
/// constructed, and return the ids that belong to a switch.
///
/// Every failure here is a wiring mistake that would otherwise surface as a
/// device that quietly answers nothing (or worse, answers from the wrong bus
/// segment). Called once from [`crate::bus::SystemBus::from_config`], which is
/// the single entry point every family's peripheral factory runs under — so
/// the per-family attach paths below can assume a valid topology.
pub fn validate_i2c_mux_topology(
    manifest: &labwired_config::SystemManifest,
) -> anyhow::Result<Vec<&str>> {
    use std::collections::HashMap;
    let by_id: HashMap<&str, &labwired_config::ExternalDevice> = manifest
        .external_devices
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();

    let mut children = Vec::new();
    for ext in &manifest.external_devices {
        let Some(parent) = by_id.get(ext.connection.as_str()) else {
            // `connection` names a controller (or nothing) — not our business.
            continue;
        };
        if parent.id == ext.id {
            anyhow::bail!(
                "external device '{}' declares itself as its own connection",
                ext.id
            );
        }
        if !is_i2c_mux_type(&parent.r#type) {
            anyhow::bail!(
                "external device '{}' hangs off '{}', but '{}' is a '{}' — only an I²C bus \
                 switch (tca9548a) can carry downstream devices",
                ext.id,
                parent.id,
                parent.id,
                parent.r#type
            );
        }
        let channel = ext.channel.unwrap_or(0);
        if channel as usize >= crate::peripherals::components::tca9548a::TCA9548A_CHANNELS {
            anyhow::bail!(
                "external device '{}' asks for channel {} of switch '{}', which has channels 0..={}",
                ext.id,
                channel,
                parent.id,
                crate::peripherals::components::tca9548a::TCA9548A_CHANNELS - 1
            );
        }
        if build_i2c_device(&ext.r#type, &ext.config).is_none() {
            // A device on a controller may legitimately fall through to the kit
            // registry or the declarative-device loader. A device behind a
            // switch has no such fallback — those paths attach straight to a
            // controller and would silently bypass the switch. Fail instead.
            anyhow::bail!(
                "external device '{}' (type '{}') sits behind I²C switch '{}', but no I²C model \
                 is registered for that type in the device factory; types reached only through \
                 the PeripheralKit registry cannot yet be placed behind a switch",
                ext.id,
                ext.r#type,
                parent.id
            );
        }
        // Walk up to the root so a `connection` cycle is a loud error rather
        // than an infinite recursion in `build_i2c_tree`.
        let mut hops = 0usize;
        let mut cursor = *parent;
        while let Some(next) = by_id.get(cursor.connection.as_str()) {
            hops += 1;
            if hops > manifest.external_devices.len() {
                anyhow::bail!(
                    "external device '{}' is in a `connection` cycle of I²C switches",
                    ext.id
                );
            }
            cursor = next;
        }
        children.push(ext.id.as_str());
    }
    Ok(children)
}

/// Build `ext`'s I²C model and, when it is a bus switch, recursively build and
/// bucket every manifest entry wired behind it onto its channels.
///
/// Returns `Ok(None)` when `ext.type` has no factory arm, exactly as
/// [`build_external_i2c_device`] does, so callers keep their existing
/// "unknown device type" handling. The returned device is ready to hand to
/// `attach_i2c_slave_with_route` as ONE unit — the switch is the thing on the
/// controller's bus; its children are not.
pub fn build_i2c_tree(
    manifest: &labwired_config::SystemManifest,
    ext: &labwired_config::ExternalDevice,
) -> anyhow::Result<Option<Box<dyn I2cDevice>>> {
    // A part this manifest CARRIES outranks the built-in factory: it is the most
    // specific thing anyone said about this system. A pack can only reach here
    // for a type we already ship by declaring `overrides:`; otherwise
    // `bus::part_pack::lookup` refuses it rather than picking a winner.
    if let Some(device) = crate::bus::part_pack::i2c_device(manifest, ext)? {
        return Ok(Some(device));
    }
    let Some(mut device) = build_external_i2c_device(&ext.r#type, &ext.id, &ext.config) else {
        return Ok(None);
    };
    let is_mux = device
        .as_any()
        .map(|a| a.is::<crate::peripherals::components::tca9548a::Tca9548a>())
        .unwrap_or(false);
    if !is_mux {
        return Ok(Some(device));
    }
    // Collect first: the recursive build borrows `manifest` immutably while
    // `device` is borrowed mutably below.
    let mut built: Vec<(u8, Box<dyn I2cDevice>)> = Vec::new();
    for child in &manifest.external_devices {
        if child.connection != ext.id {
            continue;
        }
        let Some(dev) = build_i2c_tree(manifest, child)? else {
            anyhow::bail!(
                "external device '{}' (type '{}') sits behind I²C switch '{}', but no I²C model \
                 is registered for that type",
                child.id,
                child.r#type,
                ext.id
            );
        };
        tracing::info!(
            "i2c mux attach: '{}' (type={}) -> '{}' channel {}",
            child.id,
            child.r#type,
            ext.id,
            child.channel.unwrap_or(0)
        );
        built.push((child.channel.unwrap_or(0), dev));
    }
    {
        let mux = device
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::peripherals::components::tca9548a::Tca9548a>())
            .expect("checked above that this device is a Tca9548a");
        for (channel, dev) in built {
            mux.attach(channel, dev)?;
        }
    }
    Ok(Some(device))
}

/// Build a declarative [`GenericI2cDevice`] from its embedded
/// `configs/devices/<type>.yaml` descriptor, honouring an `i2c_address` override.
/// Used by the factory arms of parts (TMP102, PCA9685) that were migrated off
/// hand-written models but still need a factory entry so every attach path — not
/// just the kit pass — wires them.
fn build_declarative_i2c_device(
    type_str: &str,
    config: &HashMap<String, serde_yaml::Value>,
) -> Option<Box<dyn I2cDevice>> {
    let yaml = labwired_config::embedded_device_yaml(type_str)?;
    // 0 tells GenericI2cDevice to use the descriptor's default_address.
    let address = config
        .get("i2c_address")
        .and_then(|v| v.as_u64())
        .map(|a| a as u8)
        .unwrap_or(0);
    match crate::peripherals::components::declarative_i2c::GenericI2cDevice::from_yaml(
        yaml, address,
    ) {
        Ok(dev) => Some(Box::new(dev)),
        Err(e) => {
            eprintln!("declarative i2c device '{type_str}': {e}");
            None
        }
    }
}

pub fn build_i2c_device(
    type_str: &str,
    config: &HashMap<String, serde_yaml::Value>,
) -> Option<Box<dyn I2cDevice>> {
    match type_str.to_ascii_lowercase().as_str() {
        // TMP102 (register-pointer + drift) and PCA9685 (byte register file +
        // servo observable) are declarative devices — the model lives entirely in
        // configs/devices/*.yaml, interpreted by the generic GenericI2cDevice. The
        // hand-written structs survive only as the byte-parity oracles.
        // The VCNL4010 joins them: its whole model is a register map plus two
        // input channels, so there is nothing for a hand-written struct to add.
        "tmp102" | "pca9685" | "vcnl4010" | "vl53l0x" => {
            build_declarative_i2c_device(&type_str.to_ascii_lowercase(), config)
        }
        "tmp117" => {
            use crate::peripherals::components::tmp117::{Tmp117, TMP117_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(TMP117_ADDR as u64) as u8;
            Some(Box::new(Tmp117::new(address)))
        }
        "mpu6050" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x68) as u8;
            Some(Box::new(crate::peripherals::components::Mpu6050::new(
                address,
            )))
        }
        "bmi270" => {
            use crate::peripherals::components::bmi270::{Bmi270, BMI270_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(BMI270_ADDR as u64) as u8;
            Some(Box::new(Bmi270::new(address)))
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
        // INA219 is also a PeripheralKit; keep a factory arm so nRF TWIM /
        // serial-instance (and any path that only calls build_external_i2c_device)
        // attach the slave — kit-only types were marked "already attached" and
        // never reached the kit pass (matrix L3 nRF ANACK).
        "ina219" => {
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .unwrap_or(0x40) as u8;
            Some(Box::new(
                crate::peripherals::components::ina219::Ina219::new(address),
            ))
        }
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
        // 8-channel I²C switch. Built EMPTY here: the devices behind it are
        // separate `external_devices` entries and are bucketed into their
        // channels by the bus loader, which is the only place that can see the
        // whole manifest. A mux built through this arm alone (a direct
        // `build_i2c_device` call, or a target whose loader has no grouping
        // pass) is a bare control register with nothing behind it — correct,
        // and visibly empty, rather than silently mis-wired.
        "tca9548a" | "pca9548a" | "tca9548" => {
            use crate::peripherals::components::tca9548a::{Tca9548a, TCA9548A_BASE_ADDR};
            let address = config
                .get("i2c_address")
                .and_then(|v| v.as_u64())
                .map(|a| a as u8)
                .unwrap_or_else(|| {
                    // No explicit address: decode the A0/A1/A2 strap pins,
                    // which is how the address is set on a real board.
                    let strap = |k: &str| config.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
                    TCA9548A_BASE_ADDR
                        | u8::from(strap("a0"))
                        | (u8::from(strap("a1")) << 1)
                        | (u8::from(strap("a2")) << 2)
                });
            Some(Box::new(Tca9548a::new(address)))
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
    fn bmi270_built_at_default_and_override_address() {
        let cfg = HashMap::new();
        assert_eq!(build_i2c_device("bmi270", &cfg).unwrap().address(), 0x68);
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x69)),
        );
        assert_eq!(build_i2c_device("bmi270", &cfg).unwrap().address(), 0x69);
    }

    #[test]
    fn tmp117_built_at_default_and_override_address() {
        let cfg = HashMap::new();
        assert_eq!(build_i2c_device("tmp117", &cfg).unwrap().address(), 0x48);
        let mut cfg = HashMap::new();
        cfg.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(serde_yaml::Number::from(0x4A)),
        );
        assert_eq!(build_i2c_device("tmp117", &cfg).unwrap().address(), 0x4A);
        // component_id stamped on the SimInput input device.
        let mut dev = build_external_i2c_device("tmp117", "tsensor", &HashMap::new()).unwrap();
        assert_eq!(
            dev.as_sim_input_mut().unwrap().component_id(),
            Some("tsensor")
        );
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
}
