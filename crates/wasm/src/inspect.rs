// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator device-state inspection accessors for the UI: board IO, ADC /
//! SPI / UART device states, display framebuffers, and peripheral listings.
//! A second #[wasm_bindgen] impl block, split out of lib.rs.

use crate::*;
use labwired_core::inspect::artifact_format as F;
use serde::Serialize;
use wasm_bindgen::prelude::*;

pub(crate) fn motor_states_json(
    snapshots: Vec<labwired_core::bus::MotorSnapshot>,
) -> Vec<serde_json::Value> {
    snapshots
        .into_iter()
        .map(|snapshot| {
            let mut state = serde_json::Map::from_iter([
                ("id".to_owned(), serde_json::json!(snapshot.id)),
                (
                    "kind".to_owned(),
                    serde_json::json!(match snapshot.kind {
                        "dc" => "dc-motor",
                        "bldc" => "bldc-motor",
                        other => other,
                    }),
                ),
                (
                    "position_rad".to_owned(),
                    serde_json::json!(snapshot.position_rad),
                ),
                (
                    "speed_rpm".to_owned(),
                    serde_json::json!(snapshot.speed_rpm),
                ),
                (
                    "torque_nm".to_owned(),
                    serde_json::json!(snapshot.torque_nm),
                ),
                (
                    "current_a".to_owned(),
                    serde_json::json!(snapshot.current_a.unwrap_or(0.0)),
                ),
                (
                    "bus_voltage_v".to_owned(),
                    serde_json::json!(snapshot.bus_voltage_v),
                ),
                (
                    "control_state".to_owned(),
                    serde_json::json!(snapshot.control_state),
                ),
                ("faults".to_owned(), serde_json::json!(snapshot.faults)),
            ]);
            if let Some(currents) = snapshot.phase_currents_a {
                state.insert("phase_currents_a".to_owned(), serde_json::json!(currents));
            }
            if let Some(sector) = snapshot.commutation_sector {
                state.insert("commutation_sector".to_owned(), serde_json::json!(sector));
            }
            serde_json::Value::Object(state)
        })
        .collect()
}


/// Both tri-color e-paper models emit this format. They are interchangeable to
/// a reader on purpose — see [`WasmSimulator::panel_artifact`] and
/// [`labwired_core::bus::SystemBus::device_artifact_at`].
const EPAPER_TRICOLOR: &[&str] = &[F::EPAPER_TRICOLOR_PLANES];

/// Rewrite integers that JavaScript cannot hold exactly as decimal STRINGS.
///
/// `serde_wasm_bindgen` does not truncate a `u64` past 2^53 — it fails the whole
/// serialization, and `to_value(..).unwrap_or(JsValue::NULL)` then hands the UI
/// a silent `null`. `meta.generation` is a 64-bit FNV hash, so it trips this
/// essentially always: `WasmSimulator::inspect` returns `null` for every machine
/// that has a device with an artifact, which is every machine with a panel.
///
/// A string keeps the value exact and keeps it comparable (`!==` is all a
/// change-detector needs), where an `f64` would silently alias two different
/// buffers to one generation and a `BigInt` would make `JSON.stringify` throw.
fn js_safe_meta(meta: &serde_json::Value) -> serde_json::Value {
    const MAX_EXACT: u64 = 1 << 53;
    match meta {
        serde_json::Value::Number(n) => match n.as_u64() {
            Some(v) if v >= MAX_EXACT => serde_json::Value::String(v.to_string()),
            _ => meta.clone(),
        },
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(js_safe_meta).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), js_safe_meta(v)))
                .collect(),
        ),
        _ => meta.clone(),
    }
}

/// How this crate reads a display: through the ONE device-evidence seam
/// `inspect` walks, never through a downcast chain of its own.
///
/// # What this replaces
///
/// Every `get_*_framebuffer` below used to re-enumerate, by hand, each I²C or
/// SPI controller its panel might hang off — `I2c`, `Esp32c3I2c`, `Esp32s3I2c`
/// for the OLEDs; `Spi`, `Esp32Spi`, `Esp32c3Spi` for the SPI panels — and then
/// downcast every attached device to one concrete model type. That is an N×M
/// matrix maintained by hand in a file far from any model, and the lists did not
/// even agree with each other: the ILI9341 accessor knew about the ESP32-C3 but
/// not the classic ESP32, the SSD1306 accessor knew about neither ESP32 SPI nor
/// the nRF52, and nothing knew about the ESP32-S3's GPSPI. A controller missing
/// from one of those lists did not fail loudly — it rendered a working,
/// actively-painted panel blank, which is exactly how the SH1107 painted a full
/// console in the browser while `inspect` reported no artifact at all.
///
/// [`labwired_core::bus::SystemBus::device_artifact_at`] is the same walk
/// `inspect` uses, so controller coverage stops being a list here and becomes a
/// property of the walk: adding a controller makes every panel readable through
/// it at once, and no panel can be visible to one surface and blind to the
/// other.
///
/// # Two identity models, reconciled deliberately
///
/// The UI addresses a panel by its `board_io` id; the seam addresses a device by
/// where it was FOUND, joining a manifest name on afterwards. Those two need not
/// agree, so only the parts of the binding that are facts about the WIRE are
/// used:
///
/// * `peripheral` and `i2c_address` — the placement — are matched against the
///   placement the walk reports. (The board_io id and the `external_devices:`
///   id are written from one placement by the compiler and are in practice the
///   same string, but nothing here depends on that: the id is only carried
///   through to stamp the artifact it addresses.)
/// * `device_type:` is deliberately NOT consulted. It is the field already
///   known to lie — a binding can say `ssd1680_tricolor_290` for a panel the
///   ESP32 builder attaches as a `Uc8151dTricolor290`, which is why
///   `get_uc8151d_framebuffer` already ignored it. What the device IS comes from
///   `meta.format`, which the model writes next to the buffer it describes.
impl WasmSimulator {
    /// THE door: whatever the display called `device_id` is showing.
    ///
    /// One resolution, tried in the order the two id registries are authoritative:
    ///
    /// 1. **The manifest name.** [`labwired_core::bus::SystemBus::display_artifact`]
    ///    joins the walk to `external_devices:`, so this reaches a display
    ///    however it was bound — I²C slave, SPI panel, a slave behind a bus
    ///    switch, or a bit-banged module that has no controller at all. This is
    ///    the step that was missing: every accessor this replaces started from
    ///    `board_io`, so a display declared only under `external_devices:` had
    ///    no placement to query and painted nothing on every chip, for every
    ///    model.
    /// 2. **The `board_io` placement**, when and only when the manifest join did
    ///    not produce this id — a model attached programmatically rather than
    ///    from a declaration (the ESP32 quirks path) has a synthesized id, and
    ///    the binding still states the wire it is on. Not a parallel system: it
    ///    is a second key into the SAME walk and the SAME
    ///    [`labwired_core::inspect::DeviceEvidence::artifacts`] call.
    fn display_artifact(
        &self,
        device_id: &str,
        include_bytes: bool,
    ) -> Option<labwired_core::inspect::Artifact> {
        let machine = self.machine.as_ref()?;
        let opts = labwired_core::inspect::InspectOpts {
            include_bytes,
            peripheral: None,
        };
        if let Some(found) = machine.bus.display_artifact(device_id, &opts) {
            return Some(found);
        }
        let binding = self.board_io.iter().find(|b| b.id == device_id)?;
        machine.bus.device_artifact_at_any(
            &binding.peripheral,
            binding.i2c_address,
            device_id,
            &opts,
        )
    }

    /// The artifact of the display bound to `device_id`, read through the seam.
    ///
    /// Kept for the per-model shims, which pass a `formats` filter so that two
    /// panels on one controller cannot be handed to each other. The door above
    /// needs no such filter: it is keyed by the id the author gave the device.
    fn panel_artifact(
        &self,
        device_id: &str,
        formats: &[&str],
        default_address: Option<u8>,
        include_bytes: bool,
        what: &str,
    ) -> Result<labwired_core::inspect::Artifact, JsValue> {
        // The one door first, so a per-model accessor inherits every route the
        // door can resolve — including the `external_devices:`-only rigs that
        // have no binding at all.
        if let Some(found) = self.display_artifact(device_id, include_bytes) {
            if found
                .meta
                .get("format")
                .and_then(|f| f.as_str())
                .is_some_and(|f| formats.contains(&f))
            {
                return Ok(found);
            }
        }
        let machine = self.machine.as_ref().unwrap();
        let binding = self
            .board_io
            .iter()
            .find(|b| b.id == device_id)
            .ok_or_else(|| JsValue::from_str(&format!("No board_io binding '{device_id}'")))?;
        // `Some(default)` is an I²C caller saying "this transport is addressed,
        // and here is the address the model defaults to when the manifest left
        // it out". `None` is an SPI caller: chip-selects were never compared.
        let address = default_address.map(|d| binding.i2c_address.unwrap_or(d));
        let opts = labwired_core::inspect::InspectOpts {
            include_bytes,
            peripheral: None,
        };
        machine
            .bus
            .device_artifact_at(&binding.peripheral, address, formats, device_id, &opts)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "{what} device not found on '{}'",
                    binding.peripheral
                ))
            })
    }

    /// The payload half of [`Self::panel_artifact`], for accessors that return
    /// raw pixels.
    fn panel_bytes(
        &self,
        device_id: &str,
        formats: &[&str],
        default_address: Option<u8>,
        what: &str,
    ) -> Result<Box<[u8]>, JsValue> {
        let artifact = self.panel_artifact(device_id, formats, default_address, true, what)?;
        artifact
            .bytes
            .map(Vec::into_boxed_slice)
            .ok_or_else(|| JsValue::from_str(&format!("{what} '{device_id}' reported no payload")))
    }

    /// One `meta` value of the panel's artifact, for accessors that return a
    /// decoded scalar or string rather than pixels. `include_bytes` stays off:
    /// nothing on this path needs the payload.
    fn panel_meta(
        &self,
        device_id: &str,
        formats: &[&str],
        default_address: Option<u8>,
        what: &str,
        key: &str,
    ) -> Result<serde_json::Value, JsValue> {
        let artifact = self.panel_artifact(device_id, formats, default_address, false, what)?;
        artifact
            .meta
            .get(key)
            .cloned()
            .ok_or_else(|| JsValue::from_str(&format!("{what} '{device_id}' reports no '{key}'")))
    }

    /// The e-paper refresh counter the UI polls before re-fetching 9472 bytes.
    /// Both tri-color panels report it under the same `meta` key, so there is
    /// one implementation rather than two that must be kept in step.
    fn refresh_generation(&self, device_id: &str, what: &str) -> Result<u32, JsValue> {
        let value =
            self.panel_meta(device_id, EPAPER_TRICOLOR, None, what, "refresh_generation")?;
        value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| JsValue::from_str("refresh_generation is not a u32"))
    }
}

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

    /// Read back NTC thermistor state from `external_devices` + live analog kit.
    ///
    /// Returns `[{ id, kind: "ntc-thermistor", divider_mv, adc_count }]`.
    /// Identity is the external_devices id (no board_io twin).
    #[wasm_bindgen]
    pub fn get_adc_device_states(&self) -> JsValue {
        use labwired_core::peripherals::adc::Adc;

        let machine = self.machine.as_ref().unwrap();
        let bus = &machine.bus;
        let mut states: Vec<serde_json::Value> = Vec::new();

        for decl in &bus.external_device_decls {
            if decl.device_type != "ntc-thermistor" {
                continue;
            }
            // Live kit model (AnalogSource) stamped with the external_devices id.
            let Some(src) = bus
                .analog_inputs
                .iter()
                .find(|a| a.source.component_id() == Some(decl.id.as_str()))
            else {
                continue;
            };
            let mv = src.source.output_mv();
            let adc_count = bus
                .find_peripheral_index_by_name(&decl.connection)
                .and_then(|idx| {
                    bus.peripherals[idx]
                        .dev
                        .as_any()
                        .and_then(|a| a.downcast_ref::<Adc>())
                        .map(|adc| adc.dr as u16)
                })
                .unwrap_or_else(|| ((u32::from(mv) * 4095) / 3300) as u16);
            states.push(serde_json::json!({
                "id": decl.id,
                "kind": "ntc-thermistor",
                "divider_mv": mv,
                "adc_count": adc_count,
            }));
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

    /// Live actuator state for canvas animation.
    ///
    /// Returns servo states followed by configured motor-plant states.
    /// - hobby servos (`kind: "servo"`) export shaft `angle` in degrees
    /// - `dc-motor` exports scalar `current_a`
    /// - `bldc-motor` exports DC-bus `current_a`, `phase_currents_a`, and
    ///   `commutation_sector`
    ///
    /// Ids match the diagram part id / external_devices id so the UI maps
    /// straight onto `boardIoStates[partId]`.
    #[wasm_bindgen]
    pub fn get_actuator_states(&self) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let mut states: Vec<serde_json::Value> = Vec::new();

        for servo in &machine.bus.servos {
            let id = servo.id();
            if id.is_empty() {
                continue;
            }
            states.push(serde_json::json!({
                "id": id,
                "kind": "servo",
                "angle": servo.angle_degrees() as f64,
                "active": servo.is_commanded(),
                "pulse_us": servo.pulse_us() as f64,
            }));
        }
        states.extend(motor_states_json(machine.bus.motor_snapshots()));

        states
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .unwrap_or(JsValue::NULL)
    }

    /// **THE door.** Whatever the display called `device_id` is showing — any
    /// model, any transport, any chip, however it was bound.
    ///
    /// Returns `null` when there is no such display, otherwise:
    ///
    /// ```text
    /// { id, kind, format, width, height, bytes, text, meta }
    /// ```
    ///
    /// * `kind` — `"framebuffer"` (packed pixels) or `"text_display"` (decoded
    ///   characters). Between them, every way this engine has of saying "a human
    ///   can see this".
    /// * `format` — how `bytes` are packed (`"rgb565_be"`, `"ssd1306_page"`,
    ///   `"epaper_tricolor_1bpp_planes"`, …).
    /// * `width` / `height` — the panel's own geometry, in pixels (or in
    ///   characters, for a text display), `null` when the model reports none.
    /// * `bytes` — the payload, present only when `include_bytes`.
    /// * `text` — the decoded string, for a `text_display`.
    /// * `meta` — everything else the model chose to report (ink counts, power
    ///   state, `generation`, `refresh_generation`), so a caller can poll for
    ///   change without pulling pixels.
    ///
    /// **Geometry and packing are DATA, deliberately.** The accessors this
    /// replaces carried them as prose in a doc comment — "153,600 bytes =
    /// 240×320×2, big-endian RGB565" — which is lore that lives in the reader.
    /// A model that arrives tomorrow cannot put anything into last year's doc
    /// comment, so every new panel needed a new accessor AND a new renderer
    /// branch before it could show a single pixel. Here a caller can paint a
    /// display it has never heard of, and a new model is renderable the day its
    /// own `artifacts()` lands.
    ///
    /// `generation` is stringified: it is a 64-bit FNV hash, and a `u64` past
    /// 2^53 makes `serde_wasm_bindgen` refuse the WHOLE payload. That is not
    /// hypothetical — it is why [`Self::inspect`] currently returns `null` for
    /// every machine that has a device with an artifact.
    #[wasm_bindgen]
    pub fn get_display(&self, device_id: &str, include_bytes: bool) -> JsValue {
        let Some(artifact) = self.display_artifact(device_id, include_bytes) else {
            return JsValue::NULL;
        };
        let meta = js_safe_meta(&artifact.meta);
        let out = serde_json::json!({
            "id": artifact.id,
            "kind": artifact.kind,
            "format": meta.get("format").cloned().unwrap_or(serde_json::Value::Null),
            "width": meta.get("w").cloned().unwrap_or(serde_json::Value::Null),
            "height": meta.get("h").cloned().unwrap_or(serde_json::Value::Null),
            "text": meta.get("text").cloned().unwrap_or(serde_json::Value::Null),
            "bytes": artifact.bytes,
            "meta": meta,
        });
        serde_wasm_bindgen::to_value(&out).unwrap_or(JsValue::NULL)
    }

    /// Return the SSD1306 GDDRAM framebuffer for the device identified by `device_id`.
    ///
    /// Returns a 1024-byte `Uint8Array` (128 columns × 8 pages, page-major) for
    /// the 128×64 panel. Both SSD1306 form factors surface through this one
    /// accessor: the framebuffer length (1024 vs 512 bytes) is what tells the
    /// renderer the panel height, so one readback path serves both.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ssd1306_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, &[F::SSD1306_PAGE], Some(0x3C), "SSD1306")
    }

    /// Return the visible text of the LCD1602 identified by `device_id`.
    ///
    /// Returns exactly 32 characters — row 0 then row 1, no separator — so the
    /// caller slices `[0..16]` and `[16..32]`. A display the firmware has not
    /// switched on reads as all spaces, matching the dark panel.
    /// Returns a JS error if the device is not found.
    ///
    /// The panel's evidence carries this text in `meta.text`, so this reads the
    /// same string the CLI and `inspect` print rather than a second decode.
    /// The default address matches the kit's own: 0x27, the PCF8574T backpack.
    #[wasm_bindgen]
    pub fn get_lcd1602_text(&self, device_id: &str) -> Result<String, JsValue> {
        let text = self.panel_meta(
            device_id,
            &[F::HD44780_DDRAM],
            Some(0x27),
            "LCD1602",
            "text",
        )?;
        text.as_str()
            .map(str::to_string)
            .ok_or_else(|| JsValue::from_str("LCD1602 text is not a string"))
    }

    /// Return the SH1107 GDDRAM framebuffer for the device identified by `device_id`.
    ///
    /// Returns a 2048-byte `Uint8Array` (128 columns × 16 pages, page-major) — the
    /// same bit layout as the SSD1306, just twice as tall (128 rows).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_sh1107_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, &[F::SH1107_PAGE], Some(0x3C), "SH1107")
    }

    /// Return the ILI9341 RGB565 framebuffer for the device identified by `device_id`.
    ///
    /// Returns a 153,600-byte `Uint8Array` (240×320 pixels × 2 bytes, row-major, big-endian RGB565).
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ili9341_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, &[F::RGB565_BE], None, "ILI9341")
    }

    /// Return the PCD8544 (Nokia 5110) framebuffer for the device identified
    /// by `device_id`.
    ///
    /// Returns 504 bytes: 84 columns × 6 banks, bank-major. Pixel (x, y) is
    /// bit `(y % 8)` of byte `[(y / 8) * 84 + x]` (1 = on/dark).
    #[wasm_bindgen]
    pub fn get_pcd8544_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, &[F::PCD8544_BANK], None, "PCD8544")
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
    /// Returns the single decoded character, with `'.'`
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
    /// Returns a 9472-byte `Uint8Array`: first 4736 bytes are the black plane
    /// (1 = white / 0 = black), next 4736 bytes are the red plane on the wire
    /// (1 = no-red / 0 = red — see GxEPD2 inversion in writeImage). Row-major,
    /// 128 pixels wide / 296 tall native, MSB-first packing within each byte.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_ssd1680_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, EPAPER_TRICOLOR, None, "SSD1680")
    }

    /// Cheap accessor returning just the SSD1680 refresh-generation counter.
    /// UI uses this to decide whether to re-fetch the (larger) framebuffer.
    #[wasm_bindgen]
    pub fn get_ssd1680_refresh_generation(&self, device_id: &str) -> Result<u32, JsValue> {
        self.refresh_generation(device_id, "SSD1680")
    }

    /// Same shape as [`Self::get_ssd1680_framebuffer`], kept as a separate name
    /// because the UI selects an accessor by the diagram part's type.
    ///
    /// It resolves to the SAME query: a tri-color e-paper on the bound
    /// controller, whichever of the two controller models the builder attached.
    /// That is not a shortcut — it is the honest reading of a `board_io`
    /// `device_type:` that says `ssd1680_tricolor_290` for a panel the ESP32
    /// builder attaches as a `Uc8151dTricolor290`. This accessor already ignored
    /// the declared type for exactly that reason; now its twin does too, so a
    /// lab can no longer render blank purely because the UI picked the accessor
    /// named after the type string rather than the one named after the model.
    #[wasm_bindgen]
    pub fn get_uc8151d_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, EPAPER_TRICOLOR, None, "UC8151D")
    }

    /// Cheap accessor returning just the UC8151D refresh-generation counter.
    #[wasm_bindgen]
    pub fn get_uc8151d_refresh_generation(&self, device_id: &str) -> Result<u32, JsValue> {
        self.refresh_generation(device_id, "UC8151D")
    }

    /// Return the MAX7219 LED-matrix framebuffer for the device identified by `device_id`.
    ///
    /// Returns an 8-byte `Uint8Array`: one byte per matrix row, row 0 first,
    /// bit 7 = the leftmost column (`SEG A` on the driver). The bytes already
    /// account for shutdown (all zero) and display test (all `0xFF`), so the
    /// renderer can paint them directly.
    /// Returns a JS error if the device is not found.
    #[wasm_bindgen]
    pub fn get_led_matrix_framebuffer(&self, device_id: &str) -> Result<Box<[u8]>, JsValue> {
        self.panel_bytes(device_id, &[F::MAX7219_ROWS], None, "MAX7219")
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
                    let Some(any) = device.as_any() else {
                        continue;
                    };
                    // The MAX31855 is a declarative kit backed by
                    // `GenericSpiDevice`; the hand-written concrete type was
                    // removed in MT5b.
                    if let Some(gen) = any
                        .downcast_ref::<labwired_core::peripherals::components::GenericSpiDevice>()
                    {
                        if let (Some(tc_c), Some(internal_c)) =
                            (gen.input_value("temperature"), gen.input_value("internal"))
                        {
                            states.push(serde_json::json!({
                                "id": binding.id,
                                "kind": "max31855",
                                "tc_c": tc_c,
                                "internal_c": internal_c,
                            }));
                            break;
                        }
                        continue;
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
    ///
    /// Two fields of that payload changed shape and the UI must handle both:
    /// `devices` (new) lists the external I²C/SPI devices the manifest placed,
    /// which are owned by their controller and so never appeared under
    /// `peripherals`; and `peripherals[].registers[].value` is now `null`
    /// rather than `0` when the model did not answer the probe, so an
    /// unmodeled-but-named register must render as unknown, not as zero.
    #[wasm_bindgen]
    pub fn inspect(&self, name: Option<String>, include_bytes: bool) -> JsValue {
        let machine = self.machine.as_ref().unwrap();
        let opts = labwired_core::inspect::InspectOpts {
            include_bytes,
            peripheral: None,
        };
        let mi = machine.inspect(name.as_deref(), &opts);
        // Round-trip through `serde_json` so `js_safe_meta` can reach the
        // artifact `generation` values. Without it this returned `null` — not
        // for a filtered peripheral, but for the WHOLE payload, on every
        // machine that has a panel: a 64-bit FNV hash is past 2^53 and
        // `serde_wasm_bindgen` refuses the entire document rather than the
        // field. Every inspect surface in the browser has been blind to
        // exactly the machines it exists to describe.
        let Ok(value) = serde_json::to_value(&mi) else {
            return JsValue::NULL;
        };
        serde_wasm_bindgen::to_value(&js_safe_meta(&value)).unwrap_or(JsValue::NULL)
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

#[cfg(test)]
mod motor_state_tests {
    use super::*;
    use labwired_core::bus::MotorSnapshot;

    #[test]
    fn motor_states_use_stable_browser_kinds_and_current_shapes() {
        let states = motor_states_json(vec![
            MotorSnapshot {
                id: "wheel".into(),
                kind: "dc",
                position_rad: 1.25,
                speed_rpm: 42.0,
                torque_nm: 0.3,
                current_a: Some(2.5),
                phase_currents_a: None,
                bus_voltage_v: 12.0,
                commutation_sector: None,
                control_state: "forward".into(),
                faults: vec!["stalled".into()],
            },
            MotorSnapshot {
                id: "spindle".into(),
                kind: "bldc",
                position_rad: 2.5,
                speed_rpm: 84.0,
                torque_nm: 0.6,
                current_a: Some(3.0),
                phase_currents_a: Some([1.0, -0.25, -0.75]),
                bus_voltage_v: 24.0,
                commutation_sector: Some(4),
                control_state: "sector:4".into(),
                faults: vec!["open-phase-a".into()],
            },
        ]);

        assert_eq!(states[0]["kind"], "dc-motor");
        assert_eq!(states[0]["current_a"], 2.5);
        assert!(states[0].get("phase_currents_a").is_none());
        assert!(states[0].get("commutation_sector").is_none());
        assert_eq!(states[1]["kind"], "bldc-motor");
        assert_eq!(states[1]["current_a"], 3.0);
        assert_eq!(
            states[1]["phase_currents_a"],
            serde_json::json!([1.0, -0.25, -0.75])
        );
        assert_eq!(states[1]["commutation_sector"], 4);
        assert_eq!(states[1]["faults"], serde_json::json!(["open-phase-a"]));
    }
}
