// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Canonical device config v1 — the loader skeleton.
//!
//! The canonical config is the ONE JSON shape the agent builds and both engines
//! load directly, collapsing today's split between a visual diagram JSON and a
//! resolved system/chip YAML manifest. It is an unresolved, Wokwi-style wiring
//! graph:
//!
//! ```json
//! {
//!   "version": 1,
//!   "parts": [
//!     { "id": "mcu",  "type": "nucleo-f401re" },
//!     { "id": "oled", "type": "ssd1306", "attrs": { "address": "0x3c" } }
//!   ],
//!   "connections": [ ["mcu:PB9","oled:SDA"], ["mcu:PB8","oled:SCL"] ]
//! }
//! ```
//!
//! The MCU is JUST a part; device config lives on a part's `attrs`, never on a
//! wire. A pin reference is `"partId:pinName"`.
//!
//! Phase 1 (this module) ships parsing + structural validation only. The
//! [`CanonicalConfig::resolve`] method — which lowers this graph into the
//! engine's [`crate::SystemManifest`] + chip descriptor — is a documented stub;
//! see its doc comment. The TS resolver (`diagramToConfig` in
//! `packages/board-config`) is the oracle Phase 2 must match byte-for-byte.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;

/// A single part in a canonical config. `attrs` carry device configuration
/// (e.g. an I2C `address`), never wiring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalPart {
    pub id: String,
    /// The catalog part type. `type` is a Rust keyword, hence `r#type` + rename.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Free-form device attributes. Absent → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Value>>,
}

/// The canonical device config, v1. A `connection` is a 2-tuple of pin
/// references; serde deserializes each `["a:b","c:d"]` JSON array into the
/// tuple and rejects any array whose length is not exactly 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalConfig {
    pub version: u32,
    pub parts: Vec<CanonicalPart>,
    pub connections: Vec<(String, String)>,
}

/// Errors from parsing or structurally validating a canonical config.
#[derive(Debug, Error)]
pub enum CanonicalError {
    /// The JSON did not parse into the canonical shape (bad JSON, unknown keys,
    /// a connection that is not a 2-element array, wrong field types, …).
    #[error("failed to parse canonical config JSON: {0}")]
    Parse(#[from] serde_json::Error),

    /// `version` is not 1.
    #[error("unsupported canonical config version {0} (expected 1)")]
    UnsupportedVersion(u32),

    /// A part `id` cannot be empty.
    #[error("part id cannot be empty")]
    EmptyPartId,

    /// A part `type` cannot be empty.
    #[error("part '{0}' has an empty type")]
    EmptyPartType(String),

    /// Two parts share an `id`.
    #[error("duplicate part id '{0}'")]
    DuplicatePartId(String),

    /// A pin reference is not of the form `partId:pin` (exactly one colon,
    /// non-empty on both sides).
    #[error("malformed pin reference '{0}' (expected 'partId:pin')")]
    MalformedPinRef(String),

    /// A connection endpoint references a part id that is not in `parts`.
    #[error("pin reference '{pin_ref}' references unknown part id '{part_id}'")]
    UnknownPart { pin_ref: String, part_id: String },

    /// The config has no part whose `type` is a known MCU (the resolver needs
    /// exactly one MCU to act as the board).
    #[error("no MCU part found (a part whose type is a known MCU deviceClass)")]
    NoMcuPart,

    /// The config has more than one MCU part; the resolver models a single MCU
    /// today (mirrors the TS `canonicalToDiagramV1`).
    #[error("multiple MCU parts ({0}); the resolver models a single MCU")]
    MultipleMcuParts(String),

    /// A part `type` is handled by a TS emitter that Phase 2a did NOT port
    /// (SPI devices, ultrasonic, pcd8544, sn74hc165, iolink-master, neo6m-gps,
    /// can-diagnostic-tool, …) — or is otherwise outside the Phase 2a subset.
    /// Phase 2b must port the remaining emitters; see the module TODO.
    #[error("part type '{0}' is not supported by the Phase 2a resolver (Phase 2b TODO)")]
    UnsupportedInPhase2a(String),
}

/// Parse a canonical config from JSON. Rejects unknown keys and malformed
/// connections at parse time (via `#[serde(deny_unknown_fields)]` and the tuple
/// deserializer). Does NOT run the semantic structural checks — call
/// [`CanonicalConfig::validate_structure`] for those.
pub fn parse_canonical(json: &str) -> Result<CanonicalConfig, CanonicalError> {
    Ok(serde_json::from_str(json)?)
}

/// Split a pin reference `"partId:pin"` into its two halves, enforcing exactly
/// one colon with non-empty sides. Mirrors the TS validator's `^[^:]+:[^:]+$`.
fn split_pin_ref(pin_ref: &str) -> Result<(&str, &str), CanonicalError> {
    match pin_ref.split_once(':') {
        Some((part, pin)) if !part.is_empty() && !pin.is_empty() && !pin.contains(':') => {
            Ok((part, pin))
        }
        _ => Err(CanonicalError::MalformedPinRef(pin_ref.to_string())),
    }
}

impl CanonicalConfig {
    /// Parse and structurally validate in one step.
    pub fn from_json(json: &str) -> Result<Self, CanonicalError> {
        let cfg = parse_canonical(json)?;
        cfg.validate_structure()?;
        Ok(cfg)
    }

    /// Structural validation: `version == 1`, non-empty & unique part ids,
    /// non-empty part types, every pin reference well-formed, and every
    /// connection endpoint pointing at a known part. This is a PURE shape check
    /// — it does not know whether a part `type` exists in a catalog or whether a
    /// pin exists on a part (those are resolver concerns).
    pub fn validate_structure(&self) -> Result<(), CanonicalError> {
        if self.version != 1 {
            return Err(CanonicalError::UnsupportedVersion(self.version));
        }

        let mut ids: HashSet<&str> = HashSet::new();
        for part in &self.parts {
            if part.id.is_empty() {
                return Err(CanonicalError::EmptyPartId);
            }
            if part.r#type.is_empty() {
                return Err(CanonicalError::EmptyPartType(part.id.clone()));
            }
            if !ids.insert(part.id.as_str()) {
                return Err(CanonicalError::DuplicatePartId(part.id.clone()));
            }
        }

        for (a, b) in &self.connections {
            for pin_ref in [a, b] {
                let (part_id, _pin) = split_pin_ref(pin_ref)?;
                if !ids.contains(part_id) {
                    return Err(CanonicalError::UnknownPart {
                        pin_ref: pin_ref.clone(),
                        part_id: part_id.to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Resolve this canonical config directly into the engine's `systemYaml`
    /// string — the Rust port of the TS oracle `diagramToConfig`
    /// (`packages/board-config/src/diagram-to-config.ts` + `compile/emitters.ts`).
    ///
    /// # Phase 2a scope
    ///
    /// This ports the subset of the TS emitters the Phase 1 parity fixtures need:
    /// legacy I²C devices ([`emit_legacy_i2c_device`], keyed by
    /// [`i2c_device_address`]) and GPIO `board_io` from point-to-point wires
    /// ([`emit_board_io_from_wires`]), assembled by [`build_system_yaml`] in the
    /// oracle's exact byte format. The MCU part (the one whose `type` is a known
    /// MCU) becomes the board; each connection's MCU pin is resolved to its
    /// peripheral ([`parse_mcu_pin`] / [`mcu_pin_for_part_pin`] /
    /// [`i2c_peripheral_for_part_wire`]).
    ///
    /// Part types handled by an emitter Phase 2a did NOT port (SPI devices,
    /// ultrasonic, pcd8544, sn74hc165, iolink-master, neo6m-gps,
    /// can-diagnostic-tool) — and any other type outside the subset — yield
    /// [`CanonicalError::UnsupportedInPhase2a`] rather than a partial result.
    ///
    /// The output is gated byte-for-byte against the TS oracle snapshots by
    /// `resolve_matches_ts_oracle` in this module's tests.
    ///
    /// ## Phase 2b TODO (unported emitters)
    ///
    /// `emitSpiDevice` (`ili9341`, `max31855`, `ssd1680_tricolor_290`),
    /// `emitUltrasonic` (`ultrasonic`), `emitPcd8544` (`pcd8544`),
    /// `emitSn74hc165` (`sn74hc165`), `emitIolinkMaster` (`iolink-master`),
    /// `emitNeo6mGps` (`neo6m-gps`), `emitCanDiagnosticTool`
    /// (`can-diagnostic-tool`), plus the ADC-input `board_io` branch (needs the
    /// per-chip ADC pin map) and non-STM32 I²C/pin maps.
    pub fn resolve(&self) -> Result<String, CanonicalError> {
        self.validate_structure()?;

        // The MCU part (catalog deviceClass 'mcu') becomes the board; the TS
        // `canonicalToDiagramV1` throws on zero or >1 MCU parts.
        let mcus: Vec<&CanonicalPart> = self
            .parts
            .iter()
            .filter(|p| MCU_TYPES.contains(&p.r#type.as_str()))
            .collect();
        match mcus.len() {
            0 => return Err(CanonicalError::NoMcuPart),
            1 => {}
            _ => {
                let ids = mcus
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CanonicalError::MultipleMcuParts(ids));
            }
        }
        let mcu = mcus[0];
        let mcu_id = mcu.id.as_str();
        // Map the MCU part `type` to the chip-family key the pin map is keyed by
        // (mirrors `mcuTypeToBoardKey`), e.g. `nucleo-f401re` → `stm32f401`.
        let board = mcu_type_to_board_key(&mcu.r#type);

        // Connections become directed wires, preserving order — the emitters are
        // order-sensitive (board_io entries are emitted in wire order).
        let wires: Vec<Wire> = self
            .connections
            .iter()
            .map(|(a, b)| {
                let (fp, fpin) = split_pin_ref(a)?;
                let (tp, tpin) = split_pin_ref(b)?;
                Ok(Wire {
                    from_part: fp.to_string(),
                    from_pin: fpin.to_string(),
                    to_part: tp.to_string(),
                    to_pin: tpin.to_string(),
                })
            })
            .collect::<Result<_, CanonicalError>>()?;

        // Fail fast on any non-MCU part outside the Phase 2a subset, rather than
        // silently dropping it (which would diverge from the oracle invisibly).
        for part in &self.parts {
            if part.id == mcu.id {
                continue;
            }
            let t = part.r#type.as_str();
            if i2c_device_address(t).is_some() {
                continue; // legacy I²C device — ported
            }
            if board_io_kind(t).is_some() {
                continue; // GPIO board_io — ported
            }
            return Err(CanonicalError::UnsupportedInPhase2a(t.to_string()));
        }

        let mut external_devices: Vec<String> = Vec::new();
        let mut board_io: Vec<String> = Vec::new();

        // Legacy I²C devices (mirrors the I2C_DEVICE_ADDRESSES loop in
        // diagramToConfig): iterate parts in order, skipping the MCU.
        for part in &self.parts {
            if part.id == mcu.id {
                continue;
            }
            let Some(address) = i2c_device_address(&part.r#type) else {
                continue;
            };
            let Some(connection) = i2c_peripheral_for_part_wire(&board, &wires, mcu_id, &part.id)
            else {
                continue;
            };
            let (ed, bio) = emit_legacy_i2c_device(&part.id, &part.r#type, &connection, address);
            external_devices.push(ed);
            board_io.push(bio);
        }

        // Wire-based board_io for the remaining (GPIO) part types.
        board_io.extend(emit_board_io_from_wires(
            &board,
            &wires,
            mcu_id,
            &self.parts,
        ));

        Ok(build_system_yaml(&external_devices, &board_io))
    }
}

// ---------------------------------------------------------------------------
// Phase 2a resolver — a faithful Rust port of the subset of the TS oracle
// (`packages/board-config/src/compile/emitters.ts` + `diagram-to-config.ts`)
// exercised by the Phase 1 parity fixtures. Gated byte-for-byte against the TS
// snapshots by `resolve_matches_ts_oracle`.
// ---------------------------------------------------------------------------

/// A directed wire (a canonical `connection` split into its two endpoints),
/// mirroring the diagram wire the TS emitters consume.
struct Wire {
    from_part: String,
    from_pin: String,
    to_part: String,
    to_pin: String,
}

/// Catalog part types whose `deviceClass` is `'mcu'` (ported from `CATALOG`).
/// One of these parts is the board.
const MCU_TYPES: &[&str] = &[
    "mcu",
    "arduino-uno",
    "stm32-dev",
    "kw41z-dev",
    "nucleo-h563zi",
    "nucleo-l476rg",
    "nucleo-f401re",
    "stm32-blackpill",
    "esp32",
    "esp32-c3-supermini",
    "esp32-s3-zero",
    "rpi-pico",
    "nrf52840-dk",
];

/// STM32 board keys whose pin map carries the F103/L476 I²C pin functions
/// (PB6–PB9 → I2C1, PB10/PB11 → I2C2). Non-STM32 I²C maps are a Phase 2b TODO.
const STM32_I2C_BOARDS: &[&str] = &[
    "stm32f103",
    "stm32f401",
    "stm32f401cdu6",
    "stm32l476",
    "stm32h563",
];

/// Map an MCU part `type` to the chip-family key the pin map is keyed by.
/// Ported from `BOARDS` (`mcuComponentType` → `chip`); falls back to the type
/// itself when no board claims it (mirrors `mcuTypeToBoardKey`).
fn mcu_type_to_board_key(mcu_type: &str) -> String {
    match mcu_type {
        "nucleo-l476rg" => "stm32l476",
        "nucleo-f401re" => "stm32f401",
        "stm32-blackpill" => "stm32f401cdu6",
        "nucleo-h563zi" => "stm32h563",
        "rpi-pico" => "rp2040",
        "nrf52840-dk" => "nrf52840",
        "esp32" => "esp32",
        "esp32-s3-zero" => "esp32s3",
        "esp32-c3-supermini" => "esp32c3",
        "stm32-dev" => "stm32f103",
        other => other,
    }
    .to_string()
}

/// Default I²C bus address for a legacy I²C device type (ported from
/// `I2C_DEVICE_ADDRESSES`). `None` → not a legacy I²C device.
fn i2c_device_address(part_type: &str) -> Option<u32> {
    match part_type {
        "adxl345" => Some(0x53),
        "mpu6050" => Some(0x68),
        "fxos8700" => Some(0x1f),
        "bme280" => Some(0x76),
        "oled-ssd1306" => Some(0x3c),
        _ => None,
    }
}

/// `board_io` kind for a GPIO part type (ported subset of `COMPONENT_META`,
/// itself derived from `CATALOG.boardIoKind`). Only the kinds the Phase 2a
/// wire-based emitter fully supports are returned; ADC-input parts are a Phase
/// 2b TODO (they need the per-chip ADC pin map) and are rejected upstream.
fn board_io_kind(part_type: &str) -> Option<&'static str> {
    match part_type {
        "led" | "rgb-led" => Some("led"),
        "button" => Some("button"),
        "pwm_output" | "servo" => Some("pwm_output"),
        _ => None,
    }
}

/// Parse an MCU pin label into `(peripheral, pin)` for various naming
/// conventions. Faithful port of `parseMcuPin`: STM32 `PA5`, Arduino `D0`/`A0`,
/// ESP32/Pico `GPIO0`/`GP0`, nRF `P0.00`. Returns `None` for power rails etc.
fn parse_mcu_pin(pin_label: &str) -> Option<(String, u32)> {
    let bytes = pin_label.as_bytes();

    // STM32: ^P([A-H])(\d+)$ (case-insensitive).
    if bytes.len() >= 3 && (bytes[0] == b'P' || bytes[0] == b'p') {
        let port = (bytes[1] as char).to_ascii_uppercase();
        if ('A'..='H').contains(&port) {
            let rest = &pin_label[2..];
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(n) = rest.parse::<u32>() {
                    return Some((format!("gpio{}", port.to_ascii_lowercase()), n));
                }
            }
        }
    }

    // Arduino: ^([DA])(\d+)$ (case-insensitive). D→gpiod, A→gpioa.
    if bytes.len() >= 2 {
        let c0 = (bytes[0] as char).to_ascii_uppercase();
        if c0 == 'D' || c0 == 'A' {
            let rest = &pin_label[1..];
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(n) = rest.parse::<u32>() {
                    let periph = if c0 == 'D' { "gpiod" } else { "gpioa" };
                    return Some((periph.to_string(), n));
                }
            }
        }
    }

    // ESP32/Pico: ^(?:GPIO|GP)(\d+)$ (case-insensitive) → gpio0.
    let lower = pin_label.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("gpio")
        .or_else(|| lower.strip_prefix("gp"))
    {
        if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = rest.parse::<u32>() {
                return Some(("gpio0".to_string(), n));
            }
        }
    }

    // nRF52840: ^P(\d+)\.(\d+)$ (case-sensitive P, per the TS regex).
    if bytes[0] == b'P' {
        if let Some(dot) = pin_label.find('.') {
            let n1 = &pin_label[1..dot];
            let n2 = &pin_label[dot + 1..];
            if !n1.is_empty()
                && n1.bytes().all(|b| b.is_ascii_digit())
                && !n2.is_empty()
                && n2.bytes().all(|b| b.is_ascii_digit())
            {
                if let (Ok(bank), Ok(pin)) = (n1.parse::<u32>(), n2.parse::<u32>()) {
                    return Some((format!("gpio{bank}"), pin));
                }
            }
        }
    }

    None
}

/// The MCU pin wired to `part_id:pin_id`, if any (port of `mcuPinForPartPin`;
/// the MCU endpoint is identified by the resolved MCU part id, which is the
/// literal `'mcu'` the TS emitters assume for the Phase 1 fixtures).
fn mcu_pin_for_part_pin<'a>(
    wires: &'a [Wire],
    mcu_id: &str,
    part_id: &str,
    pin_id: &str,
) -> Option<&'a str> {
    for w in wires {
        if w.from_part == mcu_id && w.to_part == part_id && w.to_pin == pin_id {
            return Some(&w.from_pin);
        }
        if w.to_part == mcu_id && w.from_part == part_id && w.from_pin == pin_id {
            return Some(&w.to_pin);
        }
    }
    None
}

/// The I²C peripheral for a device's SDA/SCL wire (port of
/// `i2cPeripheralForPartWire` + the STM32 `findPinFunction('i2c')` lookup).
fn i2c_peripheral_for_part_wire(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> Option<String> {
    for pin_id in ["SDA", "SCL"] {
        if let Some(mcu_pin) = mcu_pin_for_part_pin(wires, mcu_id, part_id, pin_id) {
            if let Some(peripheral) = find_i2c_peripheral(board, mcu_pin) {
                return Some(peripheral.to_string());
            }
        }
    }
    None
}

/// STM32 `findPinFunction(board, pin, 'i2c')` — the first I²C function on the
/// pin. Ported for the STM32 F103/L476 pin maps the fixtures use; other boards
/// return `None` (faithful to `findPinFunction` returning null for an unmapped
/// board), leaving the device unbound.
fn find_i2c_peripheral(board: &str, pin_label: &str) -> Option<&'static str> {
    if !STM32_I2C_BOARDS.contains(&board) {
        return None;
    }
    match pin_label.to_ascii_uppercase().as_str() {
        "PB6" | "PB7" | "PB8" | "PB9" => Some("i2c1"),
        "PB10" | "PB11" => Some("i2c2"),
        _ => None,
    }
}

/// Emit the `external_devices` + `board_io` fragments for a legacy I²C device
/// (port of `emitLegacyI2cDevice`).
fn emit_legacy_i2c_device(
    part_id: &str,
    part_type: &str,
    connection: &str,
    address: u32,
) -> (String, String) {
    let addr = format!("0x{address:x}");
    let external_device = format!(
        "  - id: \"{part_id}\"\n    type: \"{part_type}\"\n    connection: \"{connection}\"\n    config:\n      i2c_address: {addr}"
    );
    let board_io = format!(
        "  - id: \"{part_id}\"\n    kind: \"i2c_device\"\n    peripheral: \"{connection}\"\n    pin: 0\n    signal: \"input\"\n    active_high: true\n    i2c_address: {addr}\n    device_type: \"{part_type}\""
    );
    (external_device, board_io)
}

/// Emit `board_io` entries for point-to-point GPIO wires (port of
/// `emitBoardIoFromWires`). Skips parts owned by the special/I²C/SPI emitters,
/// and parts with no `board_io` kind. ADC-input peripheral remapping is a Phase
/// 2b TODO (rejected upstream), so `signal` is `input` only for buttons.
fn emit_board_io_from_wires(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    parts: &[CanonicalPart],
) -> Vec<String> {
    // Types owned by dedicated emitters (port of the TS `skipTypes` set).
    const SKIP_TYPES: &[&str] = &[
        "ultrasonic",
        "pcd8544",
        "sn74hc165",
        "iolink-master",
        "neo6m-gps",
        "can-transceiver",
        "can-diagnostic-tool",
    ];
    // SPI devices addressed by their own emitter (port of `SPI_DEVICE_TYPES`).
    const SPI_DEVICE_TYPES: &[&str] = &["ili9341", "max31855", "ssd1680_tricolor_290"];

    let _ = board; // ADC remap (the only board-dependent branch) is Phase 2b.
    let mut entries = Vec::new();

    for wire in wires {
        let (mcu_pin, comp_part): (&str, &str) = if wire.from_part == mcu_id {
            (&wire.from_pin, &wire.to_part)
        } else if wire.to_part == mcu_id {
            (&wire.to_pin, &wire.from_part)
        } else {
            continue;
        };

        let Some(part) = parts.iter().find(|p| p.id == comp_part) else {
            continue;
        };
        let t = part.r#type.as_str();
        if SKIP_TYPES.contains(&t) {
            continue;
        }
        if i2c_device_address(t).is_some() {
            continue;
        }
        if SPI_DEVICE_TYPES.contains(&t) {
            continue;
        }
        let Some(kind) = board_io_kind(t) else {
            continue;
        };
        let Some((peripheral, pin)) = parse_mcu_pin(mcu_pin) else {
            continue;
        };
        let signal = if kind == "button" { "input" } else { "output" };
        entries.push(format!(
            "  - id: \"{}\"\n    kind: \"{kind}\"\n    peripheral: \"{peripheral}\"\n    pin: {pin}\n    signal: \"{signal}\"\n    active_high: true",
            part.id
        ));
    }

    entries
}

/// Assemble the system YAML string from the fragment arrays (port of
/// `buildSystemYaml`), byte-identical including the `  []` empty-list sentinel
/// and the trailing newline.
fn build_system_yaml(external_devices: &[String], board_io: &[String]) -> String {
    let ext = if external_devices.is_empty() {
        "  []".to_string()
    } else {
        external_devices.join("\n")
    };
    let bio = if board_io.is_empty() {
        "  []".to_string()
    } else {
        board_io.join("\n")
    };
    format!("name: \"playground-board\"\nchip: \"inline\"\nexternal_devices:\n{ext}\nboard_io:\n{bio}\n")
}

#[cfg(test)]
mod canonical_tests {
    use super::*;

    const EXAMPLE: &str = r#"
    {
      "version": 1,
      "parts": [
        { "id": "mcu",  "type": "nucleo-f401re" },
        { "id": "oled", "type": "ssd1306", "attrs": { "address": "0x3c" } }
      ],
      "connections": [
        ["mcu:PB9","oled:SDA"],
        ["mcu:PB8","oled:SCL"],
        ["mcu:3V3","oled:VCC"],
        ["mcu:GND","oled:GND"]
      ]
    }
    "#;

    #[test]
    fn parses_and_validates_the_example() {
        let cfg = CanonicalConfig::from_json(EXAMPLE).expect("example is valid");
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.parts.len(), 2);
        assert_eq!(cfg.parts[0].id, "mcu");
        assert_eq!(cfg.parts[0].r#type, "nucleo-f401re");
        assert!(cfg.parts[0].attrs.is_none());
        let attrs = cfg.parts[1].attrs.as_ref().expect("oled has attrs");
        assert_eq!(attrs.get("address").unwrap(), "0x3c");
        assert_eq!(cfg.connections.len(), 4);
        assert_eq!(cfg.connections[0], ("mcu:PB9".into(), "oled:SDA".into()));
    }

    #[test]
    fn roundtrips_through_serde() {
        let cfg = CanonicalConfig::from_json(EXAMPLE).unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
        let back = parse_canonical(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let json = r#"{ "version":1, "parts":[], "connections":[], "board":"x" }"#;
        let err = parse_canonical(json).unwrap_err();
        assert!(matches!(err, CanonicalError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_unknown_part_key() {
        let json = r#"{ "version":1, "parts":[{"id":"m","type":"mcu","x":0}], "connections":[] }"#;
        let err = parse_canonical(json).unwrap_err();
        assert!(matches!(err, CanonicalError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_non_2tuple_connection() {
        let json = r#"{ "version":1, "parts":[{"id":"m","type":"mcu"}],
                        "connections":[["m:A","m:B","m:C"]] }"#;
        let err = parse_canonical(json).unwrap_err();
        assert!(matches!(err, CanonicalError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn rejects_bad_version() {
        let json = r#"{ "version":2, "parts":[], "connections":[] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(
            matches!(err, CanonicalError::UnsupportedVersion(2)),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_duplicate_part_id() {
        let json = r#"{ "version":1,
            "parts":[{"id":"m","type":"a"},{"id":"m","type":"b"}],
            "connections":[] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(
            matches!(err, CanonicalError::DuplicatePartId(ref id) if id == "m"),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_empty_part_type() {
        let json = r#"{ "version":1, "parts":[{"id":"m","type":""}], "connections":[] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(
            matches!(err, CanonicalError::EmptyPartType(ref id) if id == "m"),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_malformed_pin_ref() {
        for bad in ["mcuPB9", "mcu:", ":PB9", "a:b:c"] {
            let json = format!(
                r#"{{ "version":1, "parts":[{{"id":"mcu","type":"m"}}],
                     "connections":[["{bad}","mcu:PB8"]] }}"#
            );
            let err = CanonicalConfig::from_json(&json).unwrap_err();
            assert!(
                matches!(err, CanonicalError::MalformedPinRef(_)),
                "{bad}: got {err:?}"
            );
        }
    }

    #[test]
    fn rejects_endpoint_referencing_unknown_part() {
        let json = r#"{ "version":1, "parts":[{"id":"mcu","type":"m"}],
                        "connections":[["mcu:PB9","ghost:SDA"]] }"#;
        let err = CanonicalConfig::from_json(json).unwrap_err();
        assert!(
            matches!(err, CanonicalError::UnknownPart { ref part_id, .. } if part_id == "ghost"),
            "got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2a parity gate: the Rust `resolve()` must reproduce the TS oracle's
    // `systemYaml` byte-for-byte for the Phase 1 fixtures. The fixtures and the
    // expected outputs below are copied verbatim from
    //   packages/board-config/test/fixtures/canonical/*.json  and
    //   packages/board-config/test/__snapshots__/canonical.test.ts.snap
    // -----------------------------------------------------------------------

    const FIXTURE_I2C_OLED: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "oled", "type": "oled-ssd1306", "attrs": { "address": "0x3c" } }
      ],
      "connections": [
        ["mcu:PB9", "oled:SDA"],
        ["mcu:PB8", "oled:SCL"],
        ["mcu:3V3", "oled:VCC"],
        ["mcu:GND", "oled:GND"]
      ]
    }"#;

    const EXPECTED_I2C_OLED: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "oled"
    type: "oled-ssd1306"
    connection: "i2c1"
    config:
      i2c_address: 0x3c
board_io:
  - id: "oled"
    kind: "i2c_device"
    peripheral: "i2c1"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: 0x3c
    device_type: "oled-ssd1306"
"#;

    const FIXTURE_GPIO_LED: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-l476rg" },
        { "id": "led1", "type": "led", "attrs": { "color": "red" } }
      ],
      "connections": [
        ["mcu:PA5", "led1:A"],
        ["mcu:GND", "led1:C"]
      ]
    }"#;

    const EXPECTED_GPIO_LED: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  []
board_io:
  - id: "led1"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
"#;

    const FIXTURE_I2C_OLED_AND_LED: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "oled", "type": "oled-ssd1306", "attrs": { "address": "0x3c" } },
        { "id": "led1", "type": "led", "attrs": { "color": "green" } }
      ],
      "connections": [
        ["mcu:PB9", "oled:SDA"],
        ["mcu:PB8", "oled:SCL"],
        ["mcu:3V3", "oled:VCC"],
        ["mcu:GND", "oled:GND"],
        ["mcu:PA5", "led1:A"],
        ["mcu:GND", "led1:C"]
      ]
    }"#;

    const EXPECTED_I2C_OLED_AND_LED: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "oled"
    type: "oled-ssd1306"
    connection: "i2c1"
    config:
      i2c_address: 0x3c
board_io:
  - id: "oled"
    kind: "i2c_device"
    peripheral: "i2c1"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: 0x3c
    device_type: "oled-ssd1306"
  - id: "led1"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
"#;

    /// THE Phase 2a parity gate: for each Phase 1 fixture, the Rust `resolve()`
    /// must produce the exact `systemYaml` the TS oracle emits (byte-for-byte).
    #[test]
    fn resolve_matches_ts_oracle() {
        for (name, fixture, expected) in [
            ("i2c-oled", FIXTURE_I2C_OLED, EXPECTED_I2C_OLED),
            ("gpio-led", FIXTURE_GPIO_LED, EXPECTED_GPIO_LED),
            (
                "i2c-oled-and-led",
                FIXTURE_I2C_OLED_AND_LED,
                EXPECTED_I2C_OLED_AND_LED,
            ),
        ] {
            let cfg = CanonicalConfig::from_json(fixture)
                .unwrap_or_else(|e| panic!("{name}: fixture failed to parse/validate: {e}"));
            let yaml = cfg
                .resolve()
                .unwrap_or_else(|e| panic!("{name}: resolve() failed: {e}"));
            assert_eq!(yaml, expected, "{name}: systemYaml diverged from TS oracle");
        }
    }

    /// Part types handled by an unported Phase 2b emitter (or otherwise outside
    /// the subset) must surface a clear error, not a silently-wrong config.
    #[test]
    fn resolve_rejects_unported_part_types() {
        let json = r#"{
          "version": 1,
          "parts": [
            { "id": "mcu", "type": "nucleo-f401re" },
            { "id": "disp", "type": "ili9341" }
          ],
          "connections": [["mcu:PB9", "disp:CS"]]
        }"#;
        let cfg = CanonicalConfig::from_json(json).unwrap();
        let err = cfg.resolve().unwrap_err();
        assert!(
            matches!(err, CanonicalError::UnsupportedInPhase2a(ref t) if t == "ili9341"),
            "got {err:?}"
        );
    }

    /// A config with no MCU part is rejected (mirrors `canonicalToDiagramV1`).
    #[test]
    fn resolve_requires_an_mcu() {
        let json = r#"{ "version":1, "parts":[{"id":"led1","type":"led"}], "connections":[] }"#;
        let cfg = CanonicalConfig::from_json(json).unwrap();
        assert!(matches!(
            cfg.resolve().unwrap_err(),
            CanonicalError::NoMcuPart
        ));
    }
}
