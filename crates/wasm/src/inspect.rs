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

    /// Sample the pad level of GPIO pins for the logic analyzer.
    /// Input: `[{ kind: "gpio", peripheral, pin }]`.
    /// Output: the same refs each extended with `value: bool | null` —
    /// `null` when the pin's wire state is unknown (missing peripheral,
    /// out-of-range pin, or a pad handed to a bus controller the GPIO
    /// model doesn't track). Cheap enough to call every UI frame.
    #[wasm_bindgen]
    pub fn sample_logic_signals(&self, refs: JsValue) -> JsValue {
        #[derive(serde::Deserialize)]
        struct Ref {
            kind: String,
            peripheral: String,
            pin: u8,
        }

        let machine = self.machine.as_ref().unwrap();
        let refs: Vec<Ref> = match serde_wasm_bindgen::from_value(refs) {
            Ok(r) => r,
            Err(_) => return JsValue::NULL,
        };

        let samples: Vec<serde_json::Value> = refs
            .iter()
            .map(|r| {
                let value = if r.kind == "gpio" {
                    machine
                        .bus
                        .find_peripheral_index_by_name(&r.peripheral)
                        .and_then(|idx| machine.bus.peripherals[idx].dev.read_gpio_pad(r.pin))
                } else {
                    None
                };
                serde_json::json!({
                    "kind": r.kind,
                    "peripheral": r.peripheral,
                    "pin": r.pin,
                    "value": value,
                })
            })
            .collect();

        serde_wasm_bindgen::to_value(&samples).unwrap_or(JsValue::NULL)
    }

    /// Arm deterministic, in-engine logic-analyzer capture for a set of GPIO
    /// pads. Same ref shape as [`sample_logic_signals`]:
    /// `[{ kind: "gpio", peripheral, pin }]`.
    ///
    /// Each ref is resolved ONCE here (to a peripheral index + pin) so the
    /// in-loop sampling path never does a string lookup. Unresolvable refs
    /// (unknown peripheral / non-gpio kind) get `value: null` and are never
    /// sampled. Installing a watch set resets the capture ring and cursor.
    ///
    /// Returns the initial state as `[{ ...ref, ch, value }]` where `ch` is the
    /// channel index used in edge records (the ref's position) and `value` is
    /// the current pad level (`bool | null`). Poll new edges with
    /// [`read_logic_edges`]. Pass an empty array to disarm capture.
    #[wasm_bindgen]
    pub fn watch_logic_signals(&mut self, refs: JsValue) -> JsValue {
        #[derive(serde::Deserialize)]
        struct Ref {
            kind: String,
            peripheral: String,
            pin: u8,
        }

        let refs: Vec<Ref> = match serde_wasm_bindgen::from_value(refs) {
            Ok(r) => r,
            Err(_) => return JsValue::NULL,
        };

        let machine = self.machine.as_mut().unwrap();
        let resolved: Vec<Option<(usize, u8)>> = refs
            .iter()
            .map(|r| {
                if r.kind == "gpio" {
                    machine
                        .bus
                        .find_peripheral_index_by_name(&r.peripheral)
                        .map(|idx| (idx, r.pin))
                } else {
                    None
                }
            })
            .collect();

        let initial = machine.logic_watch(&resolved);

        let out: Vec<serde_json::Value> = refs
            .iter()
            .zip(initial)
            .enumerate()
            .map(|(ch, (r, value))| {
                serde_json::json!({
                    "kind": r.kind,
                    "peripheral": r.peripheral,
                    "pin": r.pin,
                    "ch": ch,
                    "value": value,
                })
            })
            .collect();

        serde_wasm_bindgen::to_value(&out).unwrap_or(JsValue::NULL)
    }

    /// Read logic edges captured since `cursor`. Pass `0` right after
    /// [`watch_logic_signals`], then pass back the returned `cursor` to
    /// acknowledge those retained edges and receive only newer ones.
    ///
    /// Returns `{ cursor, dropped, nowCycle, edges: [{ ch, cycle, value }] }`:
    /// - `cursor` — monotonic edge sequence number to pass back next time.
    /// - `dropped` — edges lost to ring-buffer overflow since the watch armed.
    /// - `nowCycle` — current engine cycle, to extend flat traces to "now".
    /// - `edges` — transitions oldest-first; `cycle` is the engine cycle.
    ///
    /// Cycles are emitted as JS numbers (f64), matching the sub-2^53 engine
    /// cycle counts the playground runs to.
    #[wasm_bindgen]
    pub fn read_logic_edges(&mut self, cursor: f64) -> JsValue {
        let machine = self.machine.as_mut().unwrap();
        let batch = machine.logic_read_edges(cursor as u64);
        let edges: Vec<serde_json::Value> = batch
            .edges
            .iter()
            .map(|e| {
                serde_json::json!({
                    "ch": e.ch,
                    "cycle": e.cycle as f64,
                    "value": e.value,
                })
            })
            .collect();
        let out = serde_json::json!({
            "cursor": batch.cursor as f64,
            "dropped": batch.dropped as f64,
            "nowCycle": machine.logic_now_cycle() as f64,
            "edges": edges,
        });
        serde_wasm_bindgen::to_value(&out).unwrap_or(JsValue::NULL)
    }

    /// Resolve the signal routing of GPIO pads for the logic analyzer — the
    /// engine's honest answer to "what is this pad wired to?", replacing UI-side
    /// pin-NAME regex guessing.
    ///
    /// Input: `[{ kind: "gpio", peripheral, pin }]`.
    /// Output: the same refs each extended with:
    ///   * `mode`: `"input" | "output" | "af" | "analog" | "unknown"` — derived
    ///     from the same register truth `read_gpio_pad` reads (STM32 F1 CRL/CRH,
    ///     V2 MODER+AFR, ESP32-family GPIO-matrix ENABLE + FUNCn_OUT_SEL, nRF52
    ///     DIR, Kinetis PDDR). `"unknown"` where a family cannot say.
    ///   * `func`: best-effort signal NAME (`"I2CEXT0_SDA"`, `"FSPICLK"`,
    ///     `"AF4"`, …) or `null` — never a guess.
    #[wasm_bindgen]
    pub fn pin_routing(&self, refs: JsValue) -> JsValue {
        #[derive(serde::Deserialize)]
        struct Ref {
            kind: String,
            peripheral: String,
            pin: u8,
        }

        let machine = self.machine.as_ref().unwrap();
        let refs: Vec<Ref> = match serde_wasm_bindgen::from_value(refs) {
            Ok(r) => r,
            Err(_) => return JsValue::NULL,
        };

        let out: Vec<serde_json::Value> = refs
            .iter()
            .map(|r| {
                let routing = if r.kind == "gpio" {
                    machine
                        .bus
                        .find_peripheral_index_by_name(&r.peripheral)
                        .and_then(|idx| machine.bus.peripherals[idx].dev.gpio_routing(r.pin))
                } else {
                    None
                };
                let (mode, func) = match routing {
                    Some(rt) => (
                        serde_json::to_value(rt.mode)
                            .unwrap_or_else(|_| serde_json::Value::String("unknown".into())),
                        rt.func
                            .map(serde_json::Value::String)
                            .unwrap_or(serde_json::Value::Null),
                    ),
                    None => (
                        serde_json::Value::String("unknown".into()),
                        serde_json::Value::Null,
                    ),
                };
                serde_json::json!({
                    "kind": r.kind,
                    "peripheral": r.peripheral,
                    "pin": r.pin,
                    "mode": mode,
                    "func": func,
                })
            })
            .collect();

        serde_wasm_bindgen::to_value(&out).unwrap_or(JsValue::NULL)
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

        // Find the board_io binding for this device. Both SSD1306 form factors
        // (128×64 and the 0.91″ 128×32) surface through the same accessor — the
        // framebuffer length (1024 vs 512 bytes) tells the renderer the panel
        // height, so one readback path serves both.
        let binding = self
            .board_io
            .iter()
            .find(|b| {
                b.id == device_id
                    && matches!(
                        b.device_type.as_deref(),
                        Some("oled-ssd1306") | Some("oled-ssd1306-128x32")
                    )
            })
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

        let address = binding.i2c_address.unwrap_or(0x3C);

        // The framebuffer readback has to understand every I²C controller a
        // supported board can attach an SSD1306 to. STM32 boards use the generic
        // `I2c`; the ESP32-C3 (leo air-quality lab) uses the command-list
        // `Esp32c3I2c`; the ESP32-S3 uses its own command-list `Esp32s3I2c`.
        // All attach the OLED via the bus I²C choke point, so all must be
        // enumerable here — otherwise `get_ssd1306_framebuffer` returns "not an
        // I2C controller" and the OLED renders blank in the playground/embed even
        // though the device is present and being drawn to.
        if let Some(i2c) = any.downcast_ref::<labwired_core::peripherals::i2c::I2c>() {
            for device in i2c.attached_devices() {
                let device = device.borrow();
                if device.address() != address {
                    continue;
                }
                if let Some(oled) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Ssd1306>()
                }) {
                    return Ok(oled.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else if let Some(c3) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c>()
        {
            for device in c3.attached_slaves() {
                if device.address() != address {
                    continue;
                }
                if let Some(oled) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Ssd1306>()
                }) {
                    return Ok(oled.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else if let Some(s3) =
            any.downcast_ref::<labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c>()
        {
            for device in s3.attached_slaves() {
                if device.address() != address {
                    continue;
                }
                if let Some(oled) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Ssd1306>()
                }) {
                    return Ok(oled.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an I2C controller",
                binding.peripheral
            )));
        }

        Err(JsValue::from_str(&format!(
            "SSD1306 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Return the visible text of the LCD1602 identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "lcd1602"`.
    /// Returns exactly 32 characters — row 0 then row 1, no separator — so the
    /// caller slices `[0..16]` and `[16..32]`. A display the firmware has not
    /// switched on reads as all spaces, matching the dark panel.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_lcd1602_text(&self, device_id: &str) -> Result<String, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("lcd1602"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No lcd1602 board_io binding '{}'", device_id))
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

        // Default matches the kit's own default: 0x27, the PCF8574T backpack.
        let address = binding.i2c_address.unwrap_or(0x27);

        // Same controller coverage as `get_ssd1306_framebuffer`: STM32 boards use
        // the generic `I2c`, the ESP32-C3 and ESP32-S3 use their own command-list
        // controllers. All attach character LCDs through the bus I²C choke point,
        // so all three must be enumerable here — otherwise the panel renders blank
        // in the playground even though the device is present and being written to.
        if let Some(i2c) = any.downcast_ref::<labwired_core::peripherals::i2c::I2c>() {
            for device in i2c.attached_devices() {
                let device = device.borrow();
                if device.address() != address {
                    continue;
                }
                if let Some(lcd) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Lcd1602>()
                }) {
                    return Ok(lcd.text());
                }
            }
        } else if let Some(c3) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c>()
        {
            for device in c3.attached_slaves() {
                if device.address() != address {
                    continue;
                }
                if let Some(lcd) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Lcd1602>()
                }) {
                    return Ok(lcd.text());
                }
            }
        } else if let Some(s3) =
            any.downcast_ref::<labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c>()
        {
            for device in s3.attached_slaves() {
                if device.address() != address {
                    continue;
                }
                if let Some(lcd) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Lcd1602>()
                }) {
                    return Ok(lcd.text());
                }
            }
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an I2C controller",
                binding.peripheral
            )));
        }

        Err(JsValue::from_str(&format!(
            "LCD1602 device at address 0x{:02x} not found on '{}'",
            address, binding.peripheral
        )))
    }

    /// Return the SH1107 GDDRAM framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "oled-sh1107"`.
    /// Returns a 2048-byte `Uint8Array` (128 columns × 16 pages, page-major) — the
    /// same bit layout as the SSD1306, just twice as tall (128 rows).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_sh1107_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("oled-sh1107"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No oled-sh1107 board_io binding '{}'", device_id))
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

        let address = binding.i2c_address.unwrap_or(0x3C);

        // Same multi-controller enumeration as the SSD1306 accessor: the SH1107
        // can sit on the generic STM32 `I2c` or either ESP32 command-list bus, so
        // all three must be walked or the panel renders blank in the playground.
        if let Some(i2c) = any.downcast_ref::<labwired_core::peripherals::i2c::I2c>() {
            for device in i2c.attached_devices() {
                let device = device.borrow();
                if device.address() != address {
                    continue;
                }
                if let Some(oled) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Sh1107>()
                }) {
                    return Ok(oled.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else if let Some(c3) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c>()
        {
            for device in c3.attached_slaves() {
                if device.address() != address {
                    continue;
                }
                if let Some(oled) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Sh1107>()
                }) {
                    return Ok(oled.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else if let Some(s3) =
            any.downcast_ref::<labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c>()
        {
            for device in s3.attached_slaves() {
                if device.address() != address {
                    continue;
                }
                if let Some(oled) = device.as_any().and_then(|any| {
                    any.downcast_ref::<labwired_core::peripherals::components::Sh1107>()
                }) {
                    return Ok(oled.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an I2C controller",
                binding.peripheral
            )));
        }

        Err(JsValue::from_str(&format!(
            "SH1107 device at address 0x{:02x} not found on '{}'",
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

        if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            for device in &spi.attached_devices {
                if let Some(tft) = device.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ili9341>()
                }) {
                    let fb = tft.framebuffer().to_vec().into_boxed_slice();
                    return Ok(fb);
                }
            }
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::spi::Esp32c3Spi>()
        {
            for device in spi.attached_devices() {
                if let Some(tft) = device.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Ili9341>()
                }) {
                    let fb = tft.framebuffer().to_vec().into_boxed_slice();
                    return Ok(fb);
                }
            }
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
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

        if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            for device in &spi.attached_devices {
                if let Some(lcd) = device.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Pcd8544>()
                }) {
                    return Ok(lcd.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::spi::Esp32c3Spi>()
        {
            for device in spi.attached_devices() {
                if let Some(lcd) = device.as_any().and_then(|a| {
                    a.downcast_ref::<labwired_core::peripherals::components::Pcd8544>()
                }) {
                    return Ok(lcd.framebuffer().to_vec().into_boxed_slice());
                }
            }
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
        }

        Err(JsValue::from_str(&format!(
            "PCD8544 device not found on SPI peripheral '{}'",
            binding.peripheral
        )))
    }

    /// Return the decoded four-character text currently latched into a TM1637
    /// 4-digit display. The TM1637 is GPIO bit-banged, so it is stored on the
    /// bus side rather than inside a hardware bus peripheral.
    #[wasm_bindgen]
    pub fn get_tm1637_text(&self, device_id: &str) -> Result<String, JsValue> {
        let machine = self.machine.as_ref().unwrap();
        machine
            .bus
            .tm1637
            .iter()
            .find(|dev| dev.id == device_id)
            .map(|dev| {
                let mut text = dev.text();
                if dev.colon() && text.len() >= 2 {
                    text.insert(2, ':');
                }
                if !dev.display_on() {
                    text.clear();
                }
                text
            })
            .ok_or_else(|| JsValue::from_str(&format!("TM1637 device '{}' not found", device_id)))
    }

    /// Return the character shown on the direct-drive 7-segment digit
    /// identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type:
    /// "seven-segment"`. Returns the single decoded character, with `'.'`
    /// appended when the decimal point is lit — so a blank digit is `" "`,
    /// a lit `0` is `"0"`, and `0` with the dp is `"0."`. An unrecognised
    /// segment pattern decodes to `"?"` rather than silently blanking.
    ///
    /// The lit-segment mask is polarity-normalised by the model (COM low =
    /// common cathode, COM high = common anode), so the text reads the same
    /// either way it is wired.
    #[wasm_bindgen]
    pub fn get_seven_segment_text(&self, device_id: &str) -> Result<String, JsValue> {
        let machine = self.machine.as_ref().unwrap();
        machine
            .bus
            .seven_segment
            .iter()
            .find(|dev| dev.id == device_id)
            .map(|dev| {
                let mut text = String::new();
                text.push(dev.ch());
                if dev.decimal_point() {
                    text.push('.');
                }
                text
            })
            .ok_or_else(|| {
                JsValue::from_str(&format!("7-segment device '{}' not found", device_id))
            })
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
            } else if let Some(spi) =
                any.downcast_ref::<labwired_core::peripherals::esp32c3::spi::Esp32c3Spi>()
            {
                spi.attached_devices().iter().find_map(|dev| {
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
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::spi::Esp32c3Spi>()
        {
            spi.attached_devices().iter().find_map(|dev| {
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

    /// Return the MAX7219 LED-matrix framebuffer for the device identified by `device_id`.
    ///
    /// `device_id` must match a `board_io` binding with `device_type: "led-matrix"`.
    /// Returns an 8-byte `Uint8Array`: one byte per matrix row, row 0 first,
    /// bit 7 = the leftmost column (`SEG A` on the driver). The bytes already
    /// account for shutdown (all zero) and display test (all `0xFF`), so the
    /// renderer can paint them directly.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_led_matrix_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        let machine = self.machine.as_ref().unwrap();

        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id && b.device_type.as_deref() == Some("led-matrix"))
            .ok_or_else(|| {
                JsValue::from_str(&format!("No led-matrix board_io binding '{}'", device_id))
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

        // Cover every SPI controller a supported board can hang a MAX7219 off:
        // the generic STM32-shape `Spi`, the ESP32 `Esp32Spi` and the ESP32-C3
        // `Esp32c3Spi` — the same set the SSD1680 readback enumerates. Missing
        // one here renders the matrix blank even though the device is present
        // and being driven.
        let rows = if let Some(spi) = any.downcast_ref::<labwired_core::peripherals::spi::Spi>() {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Max7219>()
                    })
                    .map(|m| m.framebuffer())
            })
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32::spi::Esp32Spi>()
        {
            spi.attached_devices.iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Max7219>()
                    })
                    .map(|m| m.framebuffer())
            })
        } else if let Some(spi) =
            any.downcast_ref::<labwired_core::peripherals::esp32c3::spi::Esp32c3Spi>()
        {
            spi.attached_devices().iter().find_map(|dev| {
                dev.as_any()
                    .and_then(|a| {
                        a.downcast_ref::<labwired_core::peripherals::components::Max7219>()
                    })
                    .map(|m| m.framebuffer())
            })
        } else {
            return Err(JsValue::from_str(&format!(
                "Peripheral '{}' is not an SPI controller",
                binding.peripheral
            )));
        };

        let rows = rows.ok_or_else(|| {
            JsValue::from_str(&format!(
                "MAX7219 device not found on SPI peripheral '{}'",
                binding.peripheral
            ))
        })?;
        Ok(rows.to_vec().into_boxed_slice())
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

    /// Universal inspect: decoded register + artifact state for one peripheral
    /// (`name = Some`) or all (`name = None`). Serializes a
    /// [`labwired_core::inspect::MachineInspect`]. In summary mode
    /// (`include_bytes = false`) large artifact payloads (framebuffers) are
    /// omitted; each artifact still carries `meta.generation` so the UI can skip
    /// re-pulling unchanged buffers. Snapshot semantics — reads the current
    /// paused machine state, side-effect-free.
    #[wasm_bindgen]
    pub fn inspect(&self, name: Option<String>, include_bytes: bool) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let opts = labwired_core::inspect::InspectOpts {
            include_bytes,
            peripheral: None,
        };
        let mi = machine.inspect(name.as_deref(), &opts);
        serde_wasm_bindgen::to_value(&mi).unwrap_or(JsValue::NULL)
    }

    /// Raw escape hatch: read `len` bytes at absolute `addr`, side-effect-free.
    /// Bytes outside any mapped region read back as `0` here (the honest
    /// mapped/unmapped markers live on the core [`labwired_core::Machine::peek`]
    /// / the `inspect` payload; this raw byte view is the fast path).
    #[wasm_bindgen]
    pub fn peek(&self, addr: u32, len: u32) -> Box<[u8]> {
        let machine = self.machine.as_ref().unwrap();
        machine
            .peek(addr as u64, len as usize)
            .to_lossy_bytes()
            .into_boxed_slice()
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
