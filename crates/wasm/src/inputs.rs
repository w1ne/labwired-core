// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator input surface. The STANDARD path is the generic trio
//! `set_input` / `set_inputs` / `list_inputs` (see `labwired_core::sim_input`)
//! — it reaches every SimInput device in engineering units; the per-device
//! setters it replaced are gone. What remains bespoke here is only what is
//! not channel-shaped yet: board_io button presses (GPIO), NTC temperature
//! (not bus-resident — seeds the ADC), raw ADC injection, UART byte feed,
//! plus the read-back queries the browser panels sync from.

use crate::*;
use wasm_bindgen::prelude::*;

#[derive(Debug, serde::Serialize)]
struct MotorControlError {
    code: &'static str,
    message: String,
    motor_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl MotorControlError {
    fn unknown_motor(id: &str) -> Self {
        Self {
            code: "unknown-motor",
            message: format!("unknown motor '{id}'"),
            motor_id: id.to_owned(),
            name: None,
        }
    }

    fn unknown_input(id: &str, name: &str) -> Self {
        Self {
            code: "unknown-input",
            message: format!("unknown motor input '{name}'"),
            motor_id: id.to_owned(),
            name: Some(name.to_owned()),
        }
    }

    fn named(code: &'static str, id: &str, name: &str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            motor_id: id.to_owned(),
            name: Some(name.to_owned()),
        }
    }
}

fn motor_error_to_js(error: MotorControlError) -> JsValue {
    serde_wasm_bindgen::to_value(&error)
        .unwrap_or_else(|_| JsValue::from_str(&format!("{}: {}", error.code, error.message)))
}

fn validate_motor_input(id: &str, name: &str, value: f64) -> Result<(), MotorControlError> {
    if !matches!(name, "load-torque-nm" | "supply-voltage-v") {
        return Err(MotorControlError::unknown_input(id, name));
    }
    if !value.is_finite() || (name == "supply-voltage-v" && value <= 0.0) {
        return Err(MotorControlError::named(
            "invalid-value",
            id,
            name,
            format!(
                "{name} must be {}",
                if name == "supply-voltage-v" {
                    "positive and finite"
                } else {
                    "finite"
                }
            ),
        ));
    }
    Ok(())
}

fn validate_motor_fault(
    kind: &str,
    id: &str,
    fault: &str,
    active: bool,
) -> Result<(), MotorControlError> {
    let known = matches!(
        fault,
        "stall"
            | "open-phase-a"
            | "open-phase-b"
            | "open-phase-c"
            | "undervoltage"
            | "overcurrent"
            | "inverter"
            | "hall-b-low"
            | "invalid-hall"
    );
    if !known {
        return Err(MotorControlError::named(
            "unknown-fault",
            id,
            fault,
            format!("unknown motor fault '{fault}'"),
        ));
    }
    if kind == "dc" && fault != "stall" {
        return Err(MotorControlError::named(
            "wrong-motor-kind",
            id,
            fault,
            format!("fault '{fault}' requires a BLDC motor"),
        ));
    }
    if fault == "overcurrent" && !active {
        return Err(MotorControlError::named(
            "unsupported-clear",
            id,
            fault,
            "overcurrent is latched and cannot be cleared",
        ));
    }
    Ok(())
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Apply one allowlisted motor-plant input in SI units.
    #[wasm_bindgen]
    pub fn set_motor_input(&mut self, id: &str, name: &str, value: f64) -> Result<(), JsValue> {
        validate_motor_input(id, name, value).map_err(motor_error_to_js)?;
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        if machine.bus.motor_kind(id).is_none() {
            return Err(motor_error_to_js(MotorControlError::unknown_motor(id)));
        }
        machine
            .bus
            .set_motor_named_input(id, name, value)
            .map_err(|message| {
                motor_error_to_js(MotorControlError::named("invalid-value", id, name, message))
            })
    }

    /// Toggle one allowlisted injected motor fault.
    #[wasm_bindgen]
    pub fn set_motor_fault(&mut self, id: &str, fault: &str, active: bool) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        let kind = machine
            .bus
            .motor_kind(id)
            .ok_or_else(|| motor_error_to_js(MotorControlError::unknown_motor(id)))?;
        validate_motor_fault(kind, id, fault, active).map_err(motor_error_to_js)?;
        machine
            .bus
            .set_motor_named_fault(id, fault, active)
            .map_err(|message| {
                motor_error_to_js(MotorControlError::named(
                    "fault-rejected",
                    id,
                    fault,
                    message,
                ))
            })
    }

    /// Generic input-scripting entry point: drive `channel` to `value` (in the
    /// channel's engineering unit — g, cm, °C …) on the unique attached input
    /// device that exposes it. Type-agnostic (see `labwired_core::sim_input`),
    /// so the browser panel, an MCP tool, and a test-script stimulus all share
    /// ONE surface. Errors if no device (or more than one) exposes the channel,
    /// or the value is out of range.
    #[wasm_bindgen]
    pub fn set_input(&mut self, channel: &str, value: f64) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        machine
            .set_input(channel, value)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Apply several input sets as ONE atomic transaction. `sets` is a JSON
    /// array of `{channel, value, component?}`; every set is validated first
    /// and either all apply or none do, with no simulation steps in between —
    /// the way to drive a multi-channel pose (an IMU's x/y/z, a GPS lat+lon)
    /// without the firmware observing a torn update, especially from a
    /// worker-engine bridge where single calls interleave with execution.
    #[wasm_bindgen]
    pub fn set_inputs(&mut self, sets: JsValue) -> Result<(), JsValue> {
        #[derive(serde::Deserialize)]
        struct InputSet {
            channel: String,
            value: f64,
            #[serde(default)]
            component: Option<String>,
        }
        let sets: Vec<InputSet> =
            serde_wasm_bindgen::from_value(sets).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        let refs: Vec<(Option<&str>, &str, f64)> = sets
            .iter()
            .map(|s| (s.component.as_deref(), s.channel.as_str(), s.value))
            .collect();
        machine
            .set_inputs(&refs)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Discover the drivable input channels on the running machine, as JSON:
    /// `[{"peripheral":"imu","key":"ax","label":"Accel X","unit":"g","min":-16,"max":16}, …]`.
    /// `peripheral` is the system.yaml external-device id when stamped (the
    /// same name `set_input`'s component selector accepts), else the owning
    /// peripheral's bus name. The "what can I drive?" query an agent calls
    /// before `set_input`.
    #[wasm_bindgen]
    pub fn list_inputs(&mut self) -> Result<JsValue, JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        let entries: Vec<serde_json::Value> = machine
            .list_inputs()
            .into_iter()
            .map(|(peripheral, ch)| {
                serde_json::json!({
                    "peripheral": peripheral,
                    "key": ch.key,
                    "label": ch.label,
                    "unit": ch.unit,
                    "min": ch.min,
                    "max": ch.max,
                })
            })
            .collect();
        serde_wasm_bindgen::to_value(&entries).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Set an input board_io binding (e.g. button press).
    /// Writes to the GPIO IDR register bit for the specified binding.
    #[wasm_bindgen]
    pub fn set_board_io_input(&mut self, id: &str, active: bool) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == id && b.signal == BoardIoSignal::Input)
            .cloned()
            .ok_or_else(|| JsValue::from_str(&format!("No input board_io binding '{}'", id)))?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!("Peripheral '{}' not found", binding.peripheral))
            })?;

        let pin_high = if binding.active_high { active } else { !active };
        if !machine.bus.peripherals[idx]
            .dev
            .set_gpio_input(binding.pin, pin_high)
        {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' does not expose GPIO input control",
                binding.peripheral
            )));
        }

        Ok(())
    }

    /// Read back live I²C sensor samples for the canvas.
    ///
    /// Identity comes from `external_devices:` (the one home for bus parts) —
    /// **not** a second `board_io` twin. Returns
    /// `[{ id, kind: "adxl345", x, y, z }, ...]` or
    /// `[{ id, kind: "mpu6050", ax, ay, az, gx, gy, gz }, ...]`.
    ///
    /// BME280 is intentionally OMITTED: its model has no register-backed
    /// engineering-unit sample API for the panel (SimInput is the stimulus path).
    #[wasm_bindgen]
    pub fn get_i2c_sensor_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let states = collect_i2c_sensor_states(&machine.bus);
        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Push bytes into all UART RX buffers (bidirectional serial input).
    #[wasm_bindgen]
    pub fn feed_uart_input(&self, data: &[u8]) {
        for buf in &self.uart_rx_bufs {
            if let Ok(mut guard) = buf.lock() {
                guard.extend(data.iter());
            }
        }
    }

    /// Inject an ADC value into a named ADC peripheral's data register.
    #[wasm_bindgen]
    pub fn set_adc_value(&mut self, peripheral_name: &str, value: u16) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(peripheral_name)
            .ok_or_else(|| JsValue::from_str(&format!("ADC '{}' not found", peripheral_name)))?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("Peripheral doesn't support downcasting"))?;
        let adc = any
            .downcast_mut::<Adc>()
            .ok_or_else(|| JsValue::from_str("Peripheral is not an ADC"))?;
        adc.dr = (value & 0xFFF) as u32;
        adc.sr |= 1 << 1; // Set EOC
        Ok(())
    }

    /// Set the simulated temperature on an NTC thermistor.
    ///
    /// `device_id` is the `external_devices` id (stamped on the kit at attach).
    /// Routes through the ONE SimInput path (`temperature` °C → kit → ADC sync).
    /// No `board_io` twin required.
    #[wasm_bindgen]
    pub fn set_ntc_temperature(
        &mut self,
        device_id: &str,
        temperature_c: f32,
    ) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        machine
            .set_input_on(device_id, "temperature", f64::from(temperature_c))
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Set the simulated wiper position on a potentiometer kit.
    ///
    /// Thin wrapper over [`Machine::set_input_on`] — identity is the
    /// `external_devices` id (no board_io twin). Kit math drives the ADC.
    /// `position_pct` must be in 0..=100.
    #[wasm_bindgen]
    pub fn set_potentiometer(&mut self, device_id: &str, position_pct: f32) -> Result<(), JsValue> {
        if !(0.0..=100.0).contains(&position_pct) {
            return Err(JsValue::from_str(&format!(
                "potentiometer position {} out of range (0..=100)",
                position_pct
            )));
        }
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        machine
            .set_input_on(device_id, "position", f64::from(position_pct))
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Read the 74HC165's live input byte (bit `i` = channel `i`), or `-1` if
    /// no shifter is wired. Lets the UI reflect the device's real state rather
    /// than tracking it in JS.
    #[wasm_bindgen]
    pub fn get_sn74hc165_inputs(&self) -> i32 {
        let machine = self.machine.as_ref().unwrap();
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else {
                continue;
            };
            let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };
            for device in &spi.attached_devices {
                if let Some(sr) = device.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Sn74hc165>()
                }) {
                    return sr.inputs() as i32;
                }
            }
        }
        -1
    }
}

/// Collect adxl345 / mpu6050 samples from `external_device_decls` + live slaves.
fn collect_i2c_sensor_states(bus: &SystemBus) -> Vec<serde_json::Value> {
    let mut states = Vec::new();
    for decl in &bus.external_device_decls {
        let kind = match decl.device_type.as_str() {
            "adxl345" | "mpu6050" => decl.device_type.as_str(),
            _ => continue,
        };
        let default_addr: u8 = if kind == "adxl345" { 0x53 } else { 0x68 };
        let address = decl.address.unwrap_or(default_addr);
        if let Some(state) = i2c_sensor_state_on_bus(bus, &decl.connection, kind, address, &decl.id)
        {
            states.push(state);
        }
    }
    states
}

/// Resolve one sensor sample from the live bus using `external_devices` identity.
///
/// Walks every known I²C controller family so a kit attached on STM32 `I2c`,
/// ESP32-C3 command-list, nRF TWIM, etc. is found the same way.
fn i2c_sensor_state_on_bus(
    bus: &SystemBus,
    connection: &str,
    kind: &str,
    address: u8,
    id: &str,
) -> Option<serde_json::Value> {
    use labwired_core::peripherals::components::{Adxl345, Mpu6050};

    let mut found: Option<serde_json::Value> = None;
    for_each_i2c_slave(bus, |ctrl_name, slave| {
        if found.is_some() {
            return;
        }
        // Prefer the declared controller; fall back to address match only when
        // the connection names a mux parent (not a peripheral) so daisy-chained
        // sensors still resolve.
        let on_declared = ctrl_name == connection;
        if !on_declared && bus.find_peripheral_index_by_name(connection).is_some() {
            return;
        }
        if slave.address() != address {
            return;
        }
        let Some(any) = slave.as_any() else {
            return;
        };
        found = match kind {
            "adxl345" => any.downcast_ref::<Adxl345>().map(|s| {
                let (x, y, z) = s.sample();
                serde_json::json!({ "id": id, "kind": "adxl345", "x": x, "y": y, "z": z })
            }),
            "mpu6050" => any.downcast_ref::<Mpu6050>().map(|s| {
                let (ax, ay, az, gx, gy, gz) = s.sample();
                serde_json::json!({
                    "id": id, "kind": "mpu6050",
                    "ax": ax, "ay": ay, "az": az, "gx": gx, "gy": gy, "gz": gz
                })
            }),
            _ => None,
        };
    });
    found
}

fn for_each_i2c_slave(
    bus: &SystemBus,
    mut f: impl FnMut(&str, &dyn labwired_core::peripherals::i2c::I2cDevice),
) {
    use labwired_core::peripherals::esp32::i2c::Esp32I2c;
    use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
    use labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c;
    use labwired_core::peripherals::i2c::I2c;
    use labwired_core::peripherals::nrf52::twim::Nrf52Twim;
    use labwired_core::peripherals::nrf54l::twim::Nrf54lTwim;
    use labwired_core::peripherals::rp2040::i2c::Rp2040I2c;

    for p in &bus.peripherals {
        let name = p.name.as_str();
        let Some(any) = p.dev.as_any() else {
            continue;
        };
        if let Some(i2c) = any.downcast_ref::<I2c>() {
            for d in i2c.attached_devices() {
                f(name, d.borrow().as_ref());
            }
        } else if let Some(i2c) = any.downcast_ref::<Esp32c3I2c>() {
            for d in i2c.attached_slaves() {
                f(name, d.as_ref());
            }
        } else if let Some(i2c) = any.downcast_ref::<Esp32s3I2c>() {
            for d in i2c.attached_slaves() {
                f(name, d.as_ref());
            }
        } else if let Some(i2c) = any.downcast_ref::<Esp32I2c>() {
            for d in i2c.attached_slaves() {
                f(name, d.as_ref());
            }
        } else if let Some(i2c) = any.downcast_ref::<Nrf52Twim>() {
            for d in i2c.attached_devices() {
                f(name, d.borrow().as_ref());
            }
        } else if let Some(i2c) = any.downcast_ref::<Nrf54lTwim>() {
            for d in i2c.attached_slaves() {
                f(name, d.as_ref());
            }
        } else if let Some(i2c) = any.downcast_ref::<Rp2040I2c>() {
            for d in i2c.attached_devices() {
                f(name, d.borrow().as_ref());
            }
        }
    }
}

#[cfg(test)]
mod motor_control_tests {
    use super::*;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;

    fn dc_bus() -> SystemBus {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: wasm-motor-test
arch: arm
core: cortex-m4
flash: { base: 0x08000000, size: "64KB" }
ram: { base: 0x20000000, size: "32KB" }
peripherals:
  - id: gpioa
    type: gpio
    base_address: 0x48000000
    size: "1KB"
    config: { profile: stm32v2 }
"#,
        )
        .unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: wasm-dc-motor
chip: unused
motor_models:
  - kind: dc
    id: wheel
    resistance_ohm: 1.0
    inductance_h: 0.001
    torque_constant_nm_per_a: 0.1
    back_emf_constant_v_per_rad_s: 0.1
    rotor_inertia_kg_m2: 0.01
    viscous_friction_nm_per_rad_s: 0.001
    supply_voltage_v: 12.0
    load_torque_nm: 0.0
    encoder_cpr: 16
    pwm_pin: PA0
    direction_pin: PA1
    brake_pin: PA2
    enable_pin: PA3
    encoder_a_pin: PA4
    encoder_b_pin: PA5
"#,
        )
        .unwrap();
        SystemBus::from_config(&chip, &manifest).unwrap()
    }

    fn bldc_bus() -> SystemBus {
        let chip: ChipDescriptor =
            serde_yaml::from_str(include_str!("../../../configs/chips/stm32l476.yaml")).unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(include_str!(
            "../../../examples/nucleo-l476rg-bldc/system.yaml"
        ))
        .unwrap();
        SystemBus::from_config(&chip, &manifest).unwrap()
    }

    #[test]
    fn motor_control_errors_are_stable_and_structured() {
        let error = MotorControlError::unknown_motor("missing");
        let json = serde_json::to_value(error).unwrap();
        assert_eq!(json["code"], "unknown-motor");
        assert_eq!(json["motor_id"], "missing");

        let error = MotorControlError::unknown_input("wheel", "raw-map-key");
        let json = serde_json::to_value(error).unwrap();
        assert_eq!(json["code"], "unknown-input");
        assert_eq!(json["name"], "raw-map-key");
    }

    #[test]
    fn motor_inputs_reject_non_finite_values_before_mutation() {
        let error = validate_motor_input("wheel", "load-torque-nm", f64::NAN).unwrap_err();
        assert_eq!(error.code, "invalid-value");
        let error = validate_motor_input("wheel", "supply-voltage-v", f64::INFINITY).unwrap_err();
        assert_eq!(error.code, "invalid-value");
    }

    #[test]
    fn motor_fault_allowlist_is_kind_specific() {
        assert!(validate_motor_fault("dc", "wheel", "stall", false).is_ok());
        let error = validate_motor_fault("dc", "wheel", "open-phase-a", true).unwrap_err();
        assert_eq!(error.code, "wrong-motor-kind");
        let error = validate_motor_fault("dc", "wheel", "hall-b-low", true).unwrap_err();
        assert_eq!(error.code, "wrong-motor-kind");
        assert!(validate_motor_fault("bldc", "spindle", "open-phase-c", false).is_ok());
        assert!(validate_motor_fault("bldc", "spindle", "hall-b-low", false).is_ok());
        assert!(validate_motor_fault("bldc", "spindle", "invalid-hall", false).is_ok());
        let error = validate_motor_fault("bldc", "spindle", "overcurrent", false).unwrap_err();
        assert_eq!(error.code, "unsupported-clear");
    }

    #[test]
    fn typed_inputs_and_faults_mutate_snapshots_and_clear_without_advancing_time() {
        let mut bus = dc_bus();
        let initial = bus.motor_snapshots();
        bus.set_motor_named_input("wheel", "supply-voltage-v", 6.0)
            .unwrap();
        assert_eq!(bus.motor_snapshots()[0].bus_voltage_v, 6.0);
        assert_eq!(
            bus.motor_snapshots()[0].position_rad,
            initial[0].position_rad,
            "input mutation must not invent elapsed simulation time"
        );

        bus.set_motor_named_fault("wheel", "stall", true).unwrap();
        assert_eq!(bus.motor_snapshots()[0].faults, ["stalled"]);
        bus.set_motor_named_fault("wheel", "stall", false).unwrap();
        assert!(bus.motor_snapshots()[0].faults.is_empty());
        assert!(bus
            .set_motor_named_input("wheel", "raw-map-key", 1.0)
            .is_err());
    }

    #[test]
    fn bldc_faults_and_inputs_round_trip_through_the_typed_core_path() {
        let mut bus = bldc_bus();
        bus.set_motor_named_input("drive_motor", "load-torque-nm", -0.02)
            .unwrap();
        bus.set_motor_named_fault("drive_motor", "open-phase-b", true)
            .unwrap();
        assert_eq!(bus.motor_snapshots()[0].faults, ["open-phase-b"]);
        bus.set_motor_named_fault("drive_motor", "open-phase-b", false)
            .unwrap();
        assert!(bus.motor_snapshots()[0].faults.is_empty());

        bus.set_motor_named_fault("drive_motor", "undervoltage", true)
            .unwrap();
        assert_eq!(bus.motor_snapshots()[0].bus_voltage_v, 12.0);
        bus.set_motor_named_fault("drive_motor", "undervoltage", false)
            .unwrap();
        assert_eq!(bus.motor_snapshots()[0].bus_voltage_v, 24.0);
    }

    #[test]
    fn bldc_open_phase_faults_are_independent_and_order_independent() {
        for (first, second) in [
            ("open-phase-a", "open-phase-b"),
            ("open-phase-b", "open-phase-a"),
        ] {
            let mut bus = bldc_bus();
            bus.set_motor_named_fault("drive_motor", first, true)
                .unwrap();
            bus.set_motor_named_fault("drive_motor", second, true)
                .unwrap();
            assert_eq!(
                bus.motor_snapshots()[0].faults,
                ["open-phase-a", "open-phase-b"]
            );

            bus.set_motor_named_fault("drive_motor", first, false)
                .unwrap();
            assert_eq!(bus.motor_snapshots()[0].faults, [second]);
            bus.set_motor_named_fault("drive_motor", first, false)
                .unwrap();
            assert_eq!(bus.motor_snapshots()[0].faults, [second]);

            bus.set_motor_named_fault("drive_motor", second, false)
                .unwrap();
            assert!(bus.motor_snapshots()[0].faults.is_empty());
        }
    }

    #[test]
    fn injected_inverter_and_hall_faults_persist_until_explicit_clear() {
        let mut bus = bldc_bus();
        bus.set_motor_named_fault("drive_motor", "inverter", true)
            .unwrap();
        bus.set_motor_named_fault("drive_motor", "hall-b-low", true)
            .unwrap();
        let before = bus.motor_snapshots();
        assert!(before[0].faults.contains(&"inverter".to_owned()));
        assert!(before[0].faults.contains(&"hall-b-low".to_owned()));

        bus.tick_peripherals_with_costs();
        assert_eq!(
            bus.motor_snapshots(),
            before,
            "zero delta must retain injected faults"
        );
        bus.set_current_cycle(100);
        bus.tick_peripherals_with_costs();
        let active = bus.motor_snapshots();
        assert!(active[0].faults.contains(&"inverter".to_owned()));
        assert!(active[0].faults.contains(&"hall-b-low".to_owned()));
        assert_eq!(active[0].control_state, "fault:inverter");
        assert_eq!(
            bus.read_u32(0x4800_0810).unwrap() & (1 << 1),
            0,
            "Hall B must be held low at the firmware GPIO"
        );

        bus.set_motor_named_fault("drive_motor", "inverter", false)
            .unwrap();
        bus.set_motor_named_fault("drive_motor", "hall-b-low", false)
            .unwrap();
        bus.set_current_cycle(200);
        bus.tick_peripherals_with_costs();
        let cleared = bus.motor_snapshots();
        assert!(!cleared[0].faults.contains(&"inverter".to_owned()));
        assert!(!cleared[0].faults.contains(&"hall-b-low".to_owned()));

        bus.set_motor_named_fault("drive_motor", "invalid-hall", true)
            .unwrap();
        bus.set_current_cycle(300);
        bus.tick_peripherals_with_costs();
        assert!(bus.motor_snapshots()[0]
            .faults
            .contains(&"invalid-hall".to_owned()));
        assert_eq!(bus.read_u32(0x4800_0810).unwrap() & 0b111, 0);
        bus.set_motor_named_fault("drive_motor", "invalid-hall", false)
            .unwrap();
    }

    #[test]
    fn wasm_snapshot_projection_is_exact_and_repeatable_for_a_fixed_trace() {
        fn run_trace() -> Vec<labwired_core::bus::MotorSnapshot> {
            let mut bus = dc_bus();
            bus.write_u32(0x4800_0014, 0b1011).unwrap();
            for cycle in [100_u64, 250, 700] {
                bus.set_current_cycle(cycle);
                bus.tick_peripherals_with_costs();
            }
            bus.motor_snapshots()
        }

        let native = run_trace();
        let repeated = run_trace();
        assert_eq!(native, repeated, "fixed traces must be bit-repeatable");
        let projected = crate::inspect::motor_states_json(native.clone());
        // Projection does no numeric conversion: native/WASM telemetry parity
        // is exact (absolute tolerance 0.0) at the public serialization seam.
        assert_eq!(projected[0]["position_rad"], native[0].position_rad);
        assert_eq!(projected[0]["speed_rpm"], native[0].speed_rpm);
        assert_eq!(projected[0]["torque_nm"], native[0].torque_nm);
        assert_eq!(projected[0]["current_a"], native[0].current_a.unwrap());
        assert_eq!(projected[0]["kind"], "dc-motor");
        assert_eq!(projected[0]["control_state"], native[0].control_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_core::cpu::riscv::RiscV;

    fn c3_button_sim() -> WasmSimulator {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_yaml = std::fs::read_to_string(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-button-test"
chip: "../chips/esp32c3.yaml"
board_io:
  - id: "left"
    kind: "button"
    peripheral: "gpio"
    pin: 2
    signal: "input"
    active_high: false
"#,
        )
        .expect("parse system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
        bus.refresh_peripheral_index();
        let machine = Machine::new(Box::new(RiscV::new()) as Box<dyn Cpu>, bus);

        WasmSimulator {
            machine: Some(machine),
            board_io: manifest.board_io,
            uart_sink: Arc::new(Mutex::new(Vec::new())),
            uart_rx_bufs: Vec::new(),
            arch: Arch::RiscV,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
        }
    }

    fn button_active(sim: &WasmSimulator) -> bool {
        let machine = sim.machine.as_ref().expect("machine");
        let binding = sim
            .board_io
            .iter()
            .find(|b| b.id == "left")
            .expect("left binding");
        sim.read_board_io_state(machine, binding)
    }

    #[test]
    fn ntc_temperature_uses_external_devices_sim_input() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_yaml =
            std::fs::read_to_string(root.join("../../configs/chips/stm32f103.yaml")).expect("chip");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "ntc-only"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "thermistor"
    type: "ntc-thermistor"
    connection: "adc1"
    config:
      channel: 0
board_io: []
"#,
        )
        .expect("manifest");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
        bus.refresh_peripheral_index();
        let mut machine = Machine::new(
            Box::new(labwired_core::cpu::cortex_m::CortexM::new()) as Box<dyn Cpu>,
            bus,
        );
        machine
            .set_input_on("thermistor", "temperature", 80.0)
            .expect("drive NTC via external_devices id");
        // Analog kit should still be addressable; channel seeded.
        assert!(
            machine
                .bus
                .analog_inputs
                .iter()
                .any(|a| a.source.component_id() == Some("thermistor")),
            "NTC kit must be stamped with external_devices id"
        );
    }

    #[test]
    fn potentiometer_uses_external_devices_sim_input() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_yaml =
            std::fs::read_to_string(root.join("../../configs/chips/stm32f103.yaml")).expect("chip");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "pot-only"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "pot1"
    type: "potentiometer"
    connection: "adc1"
    config:
      channel: 0
board_io: []
"#,
        )
        .expect("manifest");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
        bus.refresh_peripheral_index();
        let mut machine = Machine::new(
            Box::new(labwired_core::cpu::cortex_m::CortexM::new()) as Box<dyn Cpu>,
            bus,
        );
        machine
            .set_input_on("pot1", "position", 75.0)
            .expect("drive pot via external_devices id");
        assert!(
            machine
                .bus
                .analog_inputs
                .iter()
                .any(|a| a.source.component_id() == Some("pot1")),
            "pot kit must be stamped with external_devices id"
        );
    }

    #[test]
    fn i2c_sensor_states_come_from_external_devices_not_board_io() {
        // adxl345-sensor-lab shape: external_devices only (no board_io twin).
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip_yaml =
            std::fs::read_to_string(root.join("../../configs/chips/stm32f103.yaml")).expect("chip");
        let chip: ChipDescriptor = serde_yaml::from_str(&chip_yaml).expect("parse chip");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "adxl-only"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "accel"
    type: "adxl345"
    connection: "i2c1"
    config:
      i2c_address: 0x53
board_io: []
"#,
        )
        .expect("manifest");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
        bus.refresh_peripheral_index();
        let arr = collect_i2c_sensor_states(&bus);
        assert_eq!(arr.len(), 1, "expected one sensor from external_devices");
        assert_eq!(arr[0]["id"], "accel");
        assert_eq!(arr[0]["kind"], "adxl345");
        assert!(arr[0]["x"].is_number());
    }

    #[test]
    fn esp32c3_board_io_button_press_updates_gpio_input_state() {
        let mut sim = c3_button_sim();
        let machine = sim.machine.as_mut().expect("machine");
        machine
            .bus
            .write_u32(0x6000_9000 + 0x04 + 2 * 4, 1 << 8)
            .expect("enable GPIO2 FUN_WPU");

        // An active-low button with INPUT_PULLUP is released high, so it must
        // start inactive before the browser injects its first press.
        assert!(!button_active(&sim));

        sim.set_board_io_input("left", true).expect("press left");
        assert!(button_active(&sim));

        sim.set_board_io_input("left", false).expect("release left");
        assert!(!button_active(&sim));
    }
}
