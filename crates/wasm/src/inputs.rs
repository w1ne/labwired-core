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

#[wasm_bindgen]
impl WasmSimulator {
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

    /// Read back the current sensor data from each I2C sensor declared in `board_io`.
    /// Returns `[{ id, kind: "adxl345", x, y, z }, ...]` or `[{ id, kind: "mpu6050", ax, ay, az, gx, gy, gz }, ...]`.
    ///
    /// BME280 is intentionally OMITTED: its component model exposes no
    /// register-backed temperature/humidity/pressure value to read (static
    /// factory registers only, not SimInput-drivable). Rather than fabricate a
    /// number, we emit no entry so the panel shows a tracked gap.
    #[wasm_bindgen]
    pub fn get_i2c_sensor_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "adxl345" || t == "mpu6050" => t,
                _ => continue,
            };
            let Some(idx) = machine
                .bus
                .find_peripheral_index_by_name(&binding.peripheral)
            else {
                continue;
            };
            let Some(any) = machine.bus.peripherals[idx].dev.as_any() else {
                continue;
            };
            let Some(i2c) = any.downcast_ref::<labwired_core::peripherals::i2c::I2c>() else {
                continue;
            };

            if device_type == "adxl345" {
                let address = binding.i2c_address.unwrap_or(0x53);
                for device in i2c.attached_devices() {
                    let device = device.borrow();
                    if device.address() != address {
                        continue;
                    }
                    if let Some(sensor) = device.as_any().and_then(|any| {
                        any.downcast_ref::<labwired_core::peripherals::components::Adxl345>()
                    }) {
                        let (x, y, z) = sensor.sample();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "adxl345",
                            "x": x,
                            "y": y,
                            "z": z,
                        }));
                        break;
                    }
                }
            } else if device_type == "mpu6050" {
                let address = binding.i2c_address.unwrap_or(0x68);
                for device in i2c.attached_devices() {
                    let device = device.borrow();
                    if device.address() != address {
                        continue;
                    }
                    if let Some(sensor) = device.as_any().and_then(|any| {
                        any.downcast_ref::<labwired_core::peripherals::components::Mpu6050>()
                    }) {
                        let (ax, ay, az, gx, gy, gz) = sensor.sample();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "mpu6050",
                            "ax": ax,
                            "ay": ay,
                            "az": az,
                            "gx": gx,
                            "gy": gy,
                            "gz": gz,
                        }));
                        break;
                    }
                }
            }
        }

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

    /// Set the simulated temperature on an NTC thermistor attached to an ADC channel.
    ///
    /// All Steinhart-Hart math lives in Rust core (NtcThermistor::divider_output_mv).
    /// This function only stores the new temperature, recomputes divider_mv → ADC count
    /// via core, and injects the result into the ADC peripheral's channel.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "ntc-thermistor"`.
    #[wasm_bindgen]
    pub fn set_ntc_temperature(
        &mut self,
        device_id: &str,
        temperature_c: f32,
    ) -> Result<(), JsValue> {
        use labwired_core::peripherals::components::NtcThermistor;

        // Find the board_io binding for this device.
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("ntc-thermistor"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ntc-thermistor board_io binding '{}'",
                    device_id
                ))
            })?;

        let channel = binding.pin;

        // Build a temporary NTC model to compute the millivolt output — all math in core.
        let mut ntc = NtcThermistor::new(channel, temperature_c);
        ntc.set_temperature(temperature_c);
        let mv = ntc.divider_output_mv();

        // Inject the computed millivolt value into the matching ADC peripheral's channel.
        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "ADC peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("ADC peripheral does not support downcasting"))?;
        let adc = any.downcast_mut::<Adc>().ok_or_else(|| {
            JsValue::from_str(&format!(
                "Peripheral '{}' is not an ADC",
                binding.peripheral
            ))
        })?;

        adc.set_channel_input(channel, mv);
        Ok(())
    }

    /// Set the simulated wiper position on a potentiometer attached to an ADC channel.
    ///
    /// All divider math lives in Rust core (Potentiometer::wiper_output_mv).
    /// This function only validates the position, recomputes wiper_mv via core,
    /// and injects the result into the ADC peripheral's channel.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "potentiometer"`.
    /// `position_pct` must be in 0..=100.
    #[wasm_bindgen]
    pub fn set_potentiometer(
        &mut self,
        device_id: &str,
        position_pct: f32,
    ) -> Result<(), JsValue> {
        use labwired_core::peripherals::components::Potentiometer;

        if !(0.0..=100.0).contains(&position_pct) {
            return Err(JsValue::from_str(&format!(
                "potentiometer position {} out of range (0..=100)",
                position_pct
            )));
        }

        // Find the board_io binding for this device.
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("potentiometer"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No potentiometer board_io binding '{}'",
                    device_id
                ))
            })?;

        let channel = binding.pin;

        // Build a temporary pot model to compute the millivolt output — all math in core.
        let mv = Potentiometer::new(channel, position_pct).wiper_output_mv();

        // Inject the computed millivolt value into the matching ADC peripheral's channel.
        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "ADC peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("ADC peripheral does not support downcasting"))?;
        let adc = any.downcast_mut::<Adc>().ok_or_else(|| {
            JsValue::from_str(&format!(
                "Peripheral '{}' is not an ADC",
                binding.peripheral
            ))
        })?;

        adc.set_channel_input(channel, mv);
        Ok(())
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
