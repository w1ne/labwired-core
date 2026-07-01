// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator sensor / board-IO input injectors (HC-SR04, I2C sensors, ADC,
//! NTC/MAX31855, GPS, SN74HC165). Split out of lib.rs.

use crate::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl WasmSimulator {
    /// Set the distance (cm) reported by an HC-SR04 ultrasonic sensor — the
    /// host-controlled "hand position" that drives gesture control. Clamped to
    /// the sensor's 2–400 cm range.
    #[wasm_bindgen]
    pub fn set_hcsr04_distance(&mut self, id: &str, distance_cm: f32) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("simulator not initialized"))?;
        for sensor in machine.bus.hcsr04.iter_mut() {
            if sensor.id == id {
                sensor.set_distance_cm(distance_cm);
                return Ok(());
            }
        }
        Err(JsValue::from_str(&format!("No HC-SR04 sensor '{}'", id)))
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

        // Read the IDR via snapshot, modify the bit, write back via bus
        let snapshot = machine.bus.peripherals[idx].dev.snapshot();
        let current_idr = snapshot["idr"].as_u64().unwrap_or(0) as u32;

        let pin_high = if binding.active_high { active } else { !active };
        let new_idr = if pin_high {
            current_idr | (1 << binding.pin)
        } else {
            current_idr & !(1 << binding.pin)
        };

        // Write IDR through the peripheral's write interface.
        // Determine IDR offset from layout in snapshot.
        let layout = snapshot["layout"].as_str().unwrap_or("stm32_f1");
        let idr_offset: u64 = if layout.contains("v2") { 0x10 } else { 0x08 };
        let base = machine.bus.peripherals[idx].base;
        let _ = machine.bus.write_u32(base + idr_offset, new_idr);

        Ok(())
    }

    /// Set the simulated X/Y/Z sample on an ADXL345 or FXOS8700 attached to an
    /// I2C peripheral. Looks up the binding in `board_io` by id; the binding
    /// must have `device_type: "adxl345"` or `device_type: "fxos8700"`.
    #[wasm_bindgen]
    pub fn set_i2c_sensor_sample(
        &mut self,
        device_id: &str,
        x: i16,
        y: i16,
        z: i16,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| {
                b.id == device_id
                    && matches!(b.device_type.as_deref(), Some("adxl345") | Some("fxos8700"))
            })
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ADXL345/FXOS8700 board_io binding '{}'",
                    device_id
                ))
            })?;
        let device_type = binding.device_type.as_deref().unwrap_or("adxl345");

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "I2C peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("I2C peripheral does not support downcasting"))?;
        let i2c = any
            .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an I2C controller",
                    binding.peripheral
                ))
            })?;

        if device_type == "fxos8700" {
            let address = binding.i2c_address.unwrap_or(0x1f);
            for device in i2c.attached_devices() {
                let mut device = device.borrow_mut();
                if device.address() != address {
                    continue;
                }
                if let Some(sensor) = device.as_any_mut().and_then(|any| {
                    any.downcast_mut::<labwired_core::peripherals::components::Fxos8700>()
                }) {
                    sensor.set_sample(x, y, z);
                    return Ok(());
                }
            }

            return Err(JsValue::from_str(&format!(
                "FXOS8700 device at address 0x{:02x} not found on '{}'",
                address, binding.peripheral
            )));
        }

        let address = binding.i2c_address.unwrap_or(0x53);
        for device in i2c.attached_devices() {
            let mut device = device.borrow_mut();
            if device.address() != address {
                continue;
            }
            if let Some(sensor) = device.as_any_mut().and_then(|any| {
                any.downcast_mut::<labwired_core::peripherals::components::Adxl345>()
            }) {
                sensor.set_sample(x, y, z);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "ADXL345 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Set the simulated 6-DoF sample on an MPU6050 attached to an I2C peripheral.
    #[wasm_bindgen]
    #[allow(clippy::too_many_arguments)]
    pub fn set_i2c_sensor_sample_6dof(
        &mut self,
        device_id: &str,
        ax: i16,
        ay: i16,
        az: i16,
        gx: i16,
        gy: i16,
        gz: i16,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("mpu6050"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No MPU6050 board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "I2C peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("I2C peripheral does not support downcasting"))?;
        let i2c = any
            .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an I2C controller",
                    binding.peripheral
                ))
            })?;

        let address = binding.i2c_address.unwrap_or(0x68);
        for device in i2c.attached_devices() {
            let mut device = device.borrow_mut();
            if device.address() != address {
                continue;
            }
            if let Some(sensor) = device.as_any_mut().and_then(|any| {
                any.downcast_mut::<labwired_core::peripherals::components::Mpu6050>()
            }) {
                sensor.set_sample(ax, ay, az, gx, gy, gz);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "MPU6050 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Read back the current sensor data from each I2C sensor declared in `board_io`.
    /// Returns `[{ id, kind: "adxl345", x, y, z }, ...]` or `[{ id, kind: "mpu6050", ax, ay, az, gx, gy, gz }, ...]`
    /// or `[{ id, kind: "bme280", temperature_c, humidity_pct, pressure_hpa }, ...]`.
    #[wasm_bindgen]
    pub fn get_i2c_sensor_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "adxl345" || t == "mpu6050" || t == "bme280" => t,
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
            } else if device_type == "bme280" {
                // Static values: hard-coded factory calibration produces ~25°C / 50%RH / 1013hPa
                let address = binding.i2c_address.unwrap_or(0x76);
                for device in i2c.attached_devices() {
                    let device = device.borrow();
                    if device.address() != address {
                        continue;
                    }
                    if device
                        .as_any()
                        .and_then(|any| {
                            any.downcast_ref::<labwired_core::peripherals::components::Bme280>()
                        })
                        .is_some()
                    {
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "bme280",
                            "temperature_c": 25.0_f64,
                            "humidity_pct": 50.0_f64,
                            "pressure_hpa": 1013.0_f64,
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

    /// Set the simulated thermocouple and internal temperatures on a MAX31855 device.
    #[wasm_bindgen]
    pub fn set_max31855_temperature(
        &mut self,
        device_id: &str,
        tc_c: f32,
        internal_c: f32,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("max31855"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No MAX31855 board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "SPI peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("SPI peripheral does not support downcasting"))?;
        let spi = any
            .downcast_mut::<labwired_core::peripherals::spi::Spi>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                ))
            })?;

        for device in &mut spi.attached_devices {
            if let Some(sensor) = device
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<labwired_core::peripherals::components::Max31855>())
            {
                sensor.set_temperature(tc_c, internal_c);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "MAX31855 device not found on '{}'",
            binding.peripheral
        )))
    }

    /// Set the simulated position on a NEO-6M GPS module attached to a UART peripheral.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "neo6m-gps"`.
    #[wasm_bindgen]
    pub fn set_gps_position(&mut self, device_id: &str, lat: f64, lon: f64) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("neo6m-gps"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No neo6m-gps board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "UART peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("UART peripheral does not support downcasting"))?;
        let uart = any
            .downcast_mut::<labwired_core::peripherals::uart::Uart>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not a UART controller",
                    binding.peripheral
                ))
            })?;

        for stream in &mut uart.attached_streams {
            if let Some(gps) = stream
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<labwired_core::peripherals::components::Neo6mGps>())
            {
                gps.set_position(lat, lon);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "Neo6mGps not found on UART '{}'",
            binding.peripheral
        )))
    }

    /// Enable or disable the GPS fix on a NEO-6M module.
    #[wasm_bindgen]
    pub fn set_gps_fix(&mut self, device_id: &str, active: bool) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("neo6m-gps"))
            .cloned()
            .ok_or_else(|| {
                JsValue::from_str(&format!("No neo6m-gps board_io binding '{}'", device_id))
            })?;

        let machine = self.machine.as_mut().unwrap();
        let idx = machine
            .bus
            .find_peripheral_index_by_name(&binding.peripheral)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "UART peripheral '{}' not found",
                    binding.peripheral
                ))
            })?;
        let any = machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| JsValue::from_str("UART peripheral does not support downcasting"))?;
        let uart = any
            .downcast_mut::<labwired_core::peripherals::uart::Uart>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not a UART controller",
                    binding.peripheral
                ))
            })?;

        for stream in &mut uart.attached_streams {
            if let Some(gps) = stream
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<labwired_core::peripherals::components::Neo6mGps>())
            {
                gps.set_fix(active);
                return Ok(());
            }
        }

        Err(JsValue::from_str(&format!(
            "Neo6mGps not found on UART '{}'",
            binding.peripheral
        )))
    }

    /// Set all 8 digital inputs of the 74HC165 shift register at once
    /// (bit `i` = channel `i`). Returns an error if no shifter is wired.
    #[wasm_bindgen]
    pub fn set_sn74hc165_inputs(&mut self, value: u8) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(spi) = any.downcast_mut::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };
            for device in &mut spi.attached_devices {
                if let Some(sr) = device.as_any_mut().and_then(|a| {
                    a.downcast_mut::<labwired_core::peripherals::components::Sn74hc165>()
                }) {
                    sr.set_inputs(value);
                    return Ok(());
                }
            }
        }
        Err(JsValue::from_str("no 74HC165 shift register attached"))
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

    /// Toggle a single 74HC165 input channel (0..=7) high or low.
    #[wasm_bindgen]
    pub fn set_sn74hc165_channel(&mut self, channel: u8, high: bool) -> Result<(), JsValue> {
        let machine = self.machine.as_mut().unwrap();
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(spi) = any.downcast_mut::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };
            for device in &mut spi.attached_devices {
                if let Some(sr) = device.as_any_mut().and_then(|a| {
                    a.downcast_mut::<labwired_core::peripherals::components::Sn74hc165>()
                }) {
                    sr.set_channel(channel, high);
                    return Ok(());
                }
            }
        }
        Err(JsValue::from_str("no 74HC165 shift register attached"))
    }
}
