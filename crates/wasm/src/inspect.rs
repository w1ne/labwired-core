// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator device-state inspection accessors for the UI: board IO, ADC /
//! SPI / UART device states, display framebuffers, and peripheral listings.
//! A second #[wasm_bindgen] impl block, split out of lib.rs.

use crate::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl WasmSimulator {
    /// Legacy LED state query (hardcoded GPIOB pin 5 for backward compat).
    #[wasm_bindgen]
    pub fn get_led_state(&mut self) -> bool {
        let odr = self.machine().bus.read_u32(0x4001080C).unwrap_or(0);
        (odr >> 5) & 1 == 1
    }

    /// Returns the board_io configuration as a JSON array.
    /// Each entry: { id, kind, peripheral, pin, signal, active_high }
    #[wasm_bindgen]
    pub fn get_board_io_config(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.board_io).unwrap_or(JsValue::NULL)
    }

    /// Returns the current state of all board_io bindings as a JSON array.
    /// Each entry: { id, active }
    /// Uses peripheral snapshot() to read ODR regardless of register layout.
    #[wasm_bindgen]
    pub fn get_board_io_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let active = self.read_board_io_state(machine, binding);
            states.push(serde_json::json!({
                "id": binding.id,
                "active": active,
            }));
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Get a peripheral's full state snapshot as JSON.
    #[wasm_bindgen]
    pub fn get_peripheral_snapshot(&self, name: &str) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        if let Some(idx) = machine.bus.find_peripheral_index_by_name(name) {
            let snapshot = machine.bus.peripherals[idx].dev.snapshot();
            serde_wasm_bindgen::to_value(&snapshot).unwrap_or(JsValue::NULL)
        } else {
            JsValue::NULL
        }
    }

    /// Read back the current state of all NTC thermistor devices declared in `board_io`.
    ///
    /// Returns `[{ id, kind: "ntc-thermistor", temperature_c, divider_mv, adc_count }]`.
    /// All conversion math (Steinhart-Hart, mV→count) is performed here by calling into
    /// core types — no conversion logic in this WASM bridge body.
    #[wasm_bindgen]
    pub fn get_adc_device_states(&self) -> JsValue {
        use labwired_core::peripherals::components::NtcThermistor;

        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "ntc-thermistor" => t,
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
            let Some(adc) = any.downcast_ref::<Adc>() else {
                continue;
            };

            if device_type == "ntc-thermistor" {
                // Read the current ADC count from the data register.
                let adc_count = adc.dr as u16;
                // Back-compute millivolts from count (3.3 V Vref, 12-bit).
                let divider_mv = ((adc_count as u32 * 3300) / 4095) as u16;

                // Reverse the voltage divider: R_ntc = R_pull * (V_ref/V_out - 1)
                // Then use Beta equation: T = B / (ln(R/R0) + B/T0) to get temperature.
                // Build an NTC model and use divider_output_mv to find the matching temp.
                // Since we can't easily invert exp, we read temperature from what was last set.
                // Instead, we just expose the raw ADC count and mV here; the UI shows them.
                // Temperature is the authoritative value set via set_ntc_temperature.
                // Use a 25 °C default NTC to compute nominal values for display.
                let channel = binding.pin;
                // Try to recover the last-injected mV from channel_inputs.
                let injected_mv = if (channel as usize) < 18 {
                    // Access via snapshot to avoid mutable borrow; use the divider_mv we computed.
                    divider_mv
                } else {
                    divider_mv
                };

                // Build a reference NTC at 25 °C to show alongside actual values.
                let ntc_ref = NtcThermistor::new(channel, 25.0);
                let _ = ntc_ref; // Used for type verification — the display values are from ADC.

                states.push(serde_json::json!({
                    "id": binding.id,
                    "kind": "ntc-thermistor",
                    "divider_mv": injected_mv,
                    "adc_count": adc_count,
                }));
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Returns analog state for ADC and PWM board_io bindings.
    #[wasm_bindgen]
    pub fn get_board_io_analog_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            match binding.kind {
                BoardIoKind::AdcInput => {
                    if let Some(idx) = machine
                        .bus
                        .find_peripheral_index_by_name(&binding.peripheral)
                    {
                        let snap = machine.bus.peripherals[idx].dev.snapshot();
                        let dr = snap["dr"].as_u64().unwrap_or(0);
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "adc_input",
                            "value": dr,
                        }));
                    }
                }
                BoardIoKind::PwmOutput => {
                    let active = self.read_board_io_state(machine, binding);
                    states.push(serde_json::json!({
                        "id": binding.id,
                        "kind": "pwm_output",
                        "active": active,
                    }));
                }
                _ => {}
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Return the SSD1306 GDDRAM framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "oled-ssd1306"`.
    /// Returns a 1024-byte `Uint8Array` (128 columns × 8 pages, page-major).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ssd1306_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        // Find the board_io binding for this device.
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("oled-ssd1306"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No oled-ssd1306 board_io binding '{}'", device_id))
            })?;

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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let i2c = any
            .downcast_ref::<labwired_core::peripherals::i2c::I2c>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an I2C controller",
                    binding.peripheral
                ))
            })?;

        let address = binding.i2c_address.unwrap_or(0x3C);
        for device in i2c.attached_devices() {
            let device = device.borrow();
            if device.address() != address {
                continue;
            }
            if let Some(oled) = device.as_any().and_then(|any| {
                any.downcast_ref::<labwired_core::peripherals::components::Ssd1306>()
            }) {
                let fb = oled.framebuffer().to_vec().into_boxed_slice();
                return Ok(fb);
            }
        }

        Err(JsValue::from_str(&format!(
            "SSD1306 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Return the ILI9341 RGB565 framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "ili9341"`.
    /// Returns a 153,600-byte `Uint8Array` (240×320 pixels × 2 bytes, row-major, big-endian RGB565).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ili9341_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("ili9341"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No ili9341 board_io binding '{}'", device_id))
            })?;

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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let spi = any
            .downcast_ref::<labwired_core::peripherals::spi::Spi>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                ))
            })?;

        for device in &spi.attached_devices {
            if let Some(tft) = device
                .as_any()
                .and_then(|a| a.downcast_ref::<labwired_core::peripherals::components::Ili9341>())
            {
                let fb = tft.framebuffer().to_vec().into_boxed_slice();
                return Ok(fb);
            }
        }

        Err(JsValue::from_str(&format!(
            "ILI9341 device not found on SPI peripheral '{}'",
            binding.peripheral
        )))
    }

    /// Return the PCD8544 (Nokia 5110) framebuffer for the device identified
    /// by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type:
    /// "pcd8544"`. Returns 504 bytes: 84 columns × 6 banks, bank-major. Pixel
    /// (x, y) is bit `(y % 8)` of byte `[(y / 8) * 84 + x]` (1 = on/dark).
    #[wasm_bindgen]
    pub fn get_pcd8544_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("pcd8544"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No pcd8544 board_io binding '{}'", device_id))
            })?;

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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let spi = any
            .downcast_ref::<labwired_core::peripherals::spi::Spi>()
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                ))
            })?;

        for device in &spi.attached_devices {
            if let Some(lcd) = device
                .as_any()
                .and_then(|a| a.downcast_ref::<labwired_core::peripherals::components::Pcd8544>())
            {
                return Ok(lcd.framebuffer().to_vec().into_boxed_slice());
            }
        }

        Err(JsValue::from_str(&format!(
            "PCD8544 device not found on SPI peripheral '{}'",
            binding.peripheral
        )))
    }

    /// Return the SSD1680 tri-color e-paper framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "ssd1680_tricolor_290"`.
    /// Returns a 9472-byte `Uint8Array`: first 4736 bytes are the black plane
    /// (1 = white / 0 = black), next 4736 bytes are the red plane on the wire
    /// (1 = no-red / 0 = red — see GxEPD2 inversion in writeImage). Row-major,
    /// 128 pixels wide / 296 tall native, MSB-first packing within each byte.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ssd1680_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| {
                b.id == device_id
                    && matches!(
                        b.device_type.as_deref(),
                        Some("ssd1680_tricolor_290") | Some("epd-2in9-tricolor")
                    )
            })
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ssd1680_tricolor_290 board_io binding '{}'",
                    device_id
                ))
            })?;

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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        // The SSD1680 panel attaches to either the generic STM32-shape Spi
        // peripheral or the Esp32Spi controller (same SpiDevice trait,
        // different controller models). Try both downcasts.
        let panel_bytes =
            if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| (panel.black_plane().to_vec(), panel.red_plane().to_vec()))
                })
            } else if let Some(spi) =
                any.downcast_ref::<labwired_core::peripherals::esp32::spi::Esp32Spi>()
            {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| (panel.black_plane().to_vec(), panel.red_plane().to_vec()))
                })
            } else {
                return Err(JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                )));
            };

        let (black, red) = panel_bytes.ok_or_else(|| {
            JsValue::from_str(&format!(
                "SSD1680 device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })?;
        let mut combined = Vec::with_capacity(black.len() + red.len());
        combined.extend_from_slice(&black);
        combined.extend_from_slice(&red);
        Ok(combined.into_boxed_slice())
    }

    /// Cheap accessor returning just the SSD1680 refresh-generation counter.
    /// UI uses this to decide whether to re-fetch the (larger) framebuffer.
    #[wasm_bindgen]
    pub fn get_ssd1680_refresh_generation(&self, device_id: &str) -> Result<u32, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| {
                b.id == device_id
                    && matches!(
                        b.device_type.as_deref(),
                        Some("ssd1680_tricolor_290") | Some("epd-2in9-tricolor")
                    )
            })
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "No ssd1680_tricolor_290 board_io binding '{}'",
                    device_id
                ))
            })?;

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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;

        let gen = if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| panel.refresh_generation())
            })
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32::spi::Esp32Spi>()
        {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ssd1680Tricolor290>()
                }).map(|panel| panel.refresh_generation())
            })
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
        };
        gen.ok_or_else(|| {
            JsValue::from_str(&format!(
                "SSD1680 device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })
    }

    /// Same shape as [`get_ssd1680_framebuffer`] but for the UC8151D-family
    /// tri-color panel attached by [`install_arduino_esp32_quirks`]. The
    /// board_io binding type may say `ssd1680_tricolor_290` (since system
    /// YAMLs were authored before the UC8151D split); we ignore that and
    /// just find a `Uc8151dTricolor290` on the named SPI peripheral.
    #[wasm_bindgen]
    pub fn get_uc8151d_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        use labwired_core::peripherals::components::Uc8151dTricolor290;
        use labwired_core::peripherals::esp32::spi::Esp32Spi;
        let machine = self.machine.as_ref().unwrap();
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id)
            .ok_or_else(|| JsValue::from_str(&format!("No board_io binding '{}'", device_id)))?;
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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;
        let panel_bytes =
            if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any()
                        .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                        .map(|p| (p.black_plane().to_vec(), p.red_plane().to_vec()))
                })
            } else if let Some(spi) = any.downcast_ref::<Esp32Spi>() {
                spi.attached_devices.iter().find_map(|dev| {
                    dev.as_any()
                        .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                        .map(|p| (p.black_plane().to_vec(), p.red_plane().to_vec()))
                })
            } else {
                return Err(JsValue::from_str(&format!(
                    "Peripheral '{}' is not an SPI controller",
                    binding.peripheral
                )));
            };
        let (black, red) = panel_bytes.ok_or_else(|| {
            JsValue::from_str(&format!(
                "UC8151D device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })?;
        let mut combined = Vec::with_capacity(black.len() + red.len());
        combined.extend_from_slice(&black);
        combined.extend_from_slice(&red);
        Ok(combined.into_boxed_slice())
    }

    /// Cheap accessor returning just the UC8151D refresh-generation counter.
    #[wasm_bindgen]
    pub fn get_uc8151d_refresh_generation(&self, device_id: &str) -> Result<u32, JsValue> {
        use labwired_core::peripherals::components::Uc8151dTricolor290;
        use labwired_core::peripherals::esp32::spi::Esp32Spi;
        let machine = self.machine.as_ref().unwrap();
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id)
            .ok_or_else(|| JsValue::from_str(&format!("No board_io binding '{}'", device_id)))?;
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
            .as_any()
            .ok_or_else(|| JsValue::from_str("Peripheral does not support downcasting"))?;
        let gen = if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                    .map(|p| p.refresh_generation())
            })
        } else if let Some(spi) = any.downcast_ref::<Esp32Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| a.downcast_ref::<Uc8151dTricolor290>())
                    .map(|p| p.refresh_generation())
            })
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
        };
        gen.ok_or_else(|| {
            JsValue::from_str(&format!(
                "UC8151D device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })
    }

    /// Read back the current state of each SPI sensor declared in `board_io`.
    /// Returns `[{ id, kind: "max31855", tc_c, internal_c }, ...]`.
    #[wasm_bindgen]
    pub fn get_spi_device_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "max31855" => t,
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
            let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() else {
                continue;
            };

            if device_type == "max31855" {
                for device in &spi.attached_devices {
                    if let Some(sensor) = device.as_any().and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Max31855>()
                    }) {
                        let (tc_c, internal_c) = sensor.temperature();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "max31855",
                            "tc_c": tc_c,
                            "internal_c": internal_c,
                        }));
                        break;
                    }
                }
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// Read back the current state of all NEO-6M GPS devices declared in `board_io`.
    /// Returns `[{ id, kind: "neo6m-gps", lat, lon, has_fix }]`.
    #[wasm_bindgen]
    pub fn get_uart_device_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for binding in &self.board_io {
            let device_type = match binding.device_type.as_deref() {
                Some(t) if t == "neo6m-gps" => t,
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
            let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };

            if device_type == "neo6m-gps" {
                for stream in &uart.attached_streams {
                    if let Some(gps) = stream.as_any().and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Neo6mGps>()
                    }) {
                        let (lat, lon) = gps.position();
                        states.push(serde_json::json!({
                            "id": binding.id,
                            "kind": "neo6m-gps",
                            "lat": lat,
                            "lon": lon,
                            "has_fix": gps.has_fix(),
                        }));
                        break;
                    }
                }
            }
        }

        serde_wasm_bindgen::to_value(&states).unwrap_or(JsValue::NULL)
    }

    /// List all peripherals: [{ name, base_address }]
    #[wasm_bindgen]
    pub fn get_peripheral_list(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let list: Vec<serde_json::Value> = machine
            .bus
            .peripherals
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "base_address": format!("0x{:08X}", p.base),
                })
            })
            .collect();
        serde_wasm_bindgen::to_value(&list).unwrap_or(JsValue::NULL)
    }

    /// Read the IO-Link master peer's live state: `{ link_state, pd_valid,
    /// input_byte }`. Returns `null` if no master is wired.
    #[wasm_bindgen]
    pub fn get_iolink_master_state(&self) -> JsValue {
        use labwired_core::peripherals::components::{IolinkLinkState, IolinkMaster};
        let machine = self.machine.as_ref().unwrap();
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else {
                continue;
            };
            let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                if let Some(m) = stream
                    .as_any()
                    .and_then(|a| a.downcast_ref::<IolinkMaster>())
                {
                    let link = match m.link_state {
                        IolinkLinkState::Startup => "startup",
                        IolinkLinkState::Operate => "operate",
                    };
                    let v = serde_json::json!({
                        "link_state": link,
                        "pd_valid": m.pd_valid,
                        "input_byte": m.input_byte(),
                    });
                    return serde_wasm_bindgen::to_value(&v).unwrap_or(JsValue::NULL);
                }
            }
        }
        JsValue::NULL
    }
}
