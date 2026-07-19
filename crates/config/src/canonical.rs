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
    /// The MCU part (the one whose `type` is a known MCU) becomes the board; the
    /// TS `canonicalToDiagramV1` throws on zero or >1 MCU parts, so this returns
    /// [`CanonicalError::NoMcuPart`] / [`CanonicalError::MultipleMcuParts`] to
    /// match. Every `connection`'s MCU pin is resolved to its peripheral via the
    /// ported pin maps ([`parse_mcu_pin`] / [`mcu_pin_for_part_pin`] / the
    /// `find_*_peripheral` lookups).
    ///
    /// # Ported emitters (Phase 2a + 2b)
    ///
    /// All TS emitters are ported: legacy I²C devices ([`emit_legacy_i2c_device`]),
    /// GPIO/ADC `board_io` from point-to-point wires ([`emit_board_io_from_wires`]),
    /// SPI devices ([`emit_spi_device`]), ultrasonic ([`emit_ultrasonic`]),
    /// the direct-drive seven-segment ([`emit_seven_segment`]),
    /// pcd8544 ([`emit_pcd8544`]), sn74hc165 ([`emit_sn74hc165`]), iolink-master
    /// ([`emit_iolink_master`]), neo6m-gps ([`emit_neo6m_gps`]) and the CAN
    /// diagnostic tester ([`emit_can_diagnostic_tool`]). They are invoked in the
    /// exact order `diagramToConfig` uses (the output is byte-order-sensitive) and
    /// assembled by [`build_system_yaml`].
    ///
    /// Like the TS oracle, `resolve()` is total over part types: a part no emitter
    /// claims (a passive/tool bridge part such as `can-transceiver`, or an unknown
    /// type) simply contributes nothing — it is never an error.
    ///
    /// # Bus pin-map coverage
    ///
    /// The bus lookups ([`find_i2c_peripheral`], [`find_spi_peripheral`],
    /// [`find_uart_peripheral`], [`find_can_function`], [`find_adc_peripheral`])
    /// cover the STM32 F103/F401/L476/H563 pin maps — the only chips the TS oracle
    /// can actually resolve, since `diagramToConfig` throws for any board absent
    /// from `CHIP_YAMLS` (currently `stm32f103`/`stm32f401`/`stm32l476`). Non-STM32
    /// boards resolve GPIO `board_io` (via [`parse_mcu_pin`]) but return `None`
    /// from the bus lookups, so a bus device on them stays unbound — faithful to
    /// `findPinFunction` returning null for an unmapped board.
    ///
    /// The output is gated byte-for-byte against the TS oracle snapshots by
    /// `resolve_matches_ts_oracle` in this module's tests.
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

        let mut external_devices: Vec<String> = Vec::new();
        let mut board_io: Vec<String> = Vec::new();

        // Non-MCU parts, in original order (mirrors `diagram.parts`, which
        // `canonicalToDiagramV1` builds by filtering out the MCU).
        let non_mcu = || self.parts.iter().filter(|p| p.id != mcu.id);

        // The dedicated-emitter loops run in the SAME order as `diagramToConfig`,
        // because both the `external_devices` and `board_io` lists are emitted in
        // this order (byte-order matters).
        {
            let mut push = |ext: Option<String>, bio: Option<String>| {
                if let Some(e) = ext {
                    external_devices.push(e);
                }
                if let Some(b) = bio {
                    board_io.push(b);
                }
            };

            // 1. Ultrasonic (HC-SR04).
            for part in non_mcu() {
                if part.r#type == "ultrasonic" {
                    let (ext, bio) = emit_ultrasonic(&board, &wires, mcu_id, part);
                    push(ext, bio);
                }
            }
            // 1b. Direct-drive seven-segment (nine GPIO pins).
            for part in non_mcu() {
                if part.r#type == "seven-segment" {
                    let (ext, bio) = emit_seven_segment(&wires, mcu_id, &part.id);
                    push(ext, bio);
                }
            }
            // 2. PCD8544 (Nokia 5110).
            for part in non_mcu() {
                if part.r#type == "pcd8544" {
                    let (ext, bio) = emit_pcd8544(&board, &wires, mcu_id, &part.id);
                    push(ext, bio);
                }
            }
            // 3. SN74HC165 shift register.
            for part in non_mcu() {
                if part.r#type == "sn74hc165" {
                    let (ext, bio) = emit_sn74hc165(&board, &wires, mcu_id, part);
                    push(ext, bio);
                }
            }
            // 4. IO-Link master.
            for part in non_mcu() {
                if part.r#type == "iolink-master" {
                    let (ext, bio) =
                        emit_iolink_master(&board, &wires, mcu_id, &self.parts, &part.id);
                    push(ext, bio);
                }
            }
            // 5. Legacy I²C devices (adxl345, mpu6050, bme280, oled-ssd1306, …).
            for part in non_mcu() {
                let Some(address) = i2c_device_address(&part.r#type) else {
                    continue;
                };
                let Some(connection) =
                    i2c_peripheral_for_part_wire(&board, &wires, mcu_id, &part.id)
                else {
                    continue;
                };
                let (ed, bio) =
                    emit_legacy_i2c_device(&part.id, &part.r#type, &connection, address);
                push(Some(ed), Some(bio));
            }
            // 6. SPI devices (ili9341, max31855, ssd1680_tricolor_290).
            for part in non_mcu() {
                if SPI_DEVICE_TYPES.contains(&part.r#type.as_str()) {
                    let (ext, bio) =
                        emit_spi_device(&board, &wires, mcu_id, &part.id, &part.r#type);
                    push(ext, bio);
                }
            }
            // 7. NEO-6M GPS.
            for part in non_mcu() {
                if part.r#type == "neo6m-gps" {
                    let (ext, bio) = emit_neo6m_gps(&board, &wires, mcu_id, &part.id);
                    push(ext, bio);
                }
            }
            // 8. CAN diagnostic tester.
            for part in non_mcu() {
                if part.r#type == "can-diagnostic-tool" {
                    let (ext, bio) =
                        emit_can_diagnostic_tool(&board, &wires, mcu_id, &self.parts, part);
                    push(ext, bio);
                }
            }
        }

        // 9. Wire-based board_io for all remaining (GPIO/ADC) part types.
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

/// STM32 board keys whose pin map carries the F103/L476/F401/H563 bus pin
/// functions (I²C on PB6–PB11, SPI on PA/PB/PC, UART on PA/PB/PC, CAN on
/// PA11/PA12, ADC on PA/PB/PC). All five share the F103 baseline for the pins
/// the emitters resolve (L476 differs only in ADC *channel* numbers, which the
/// emitters never emit — only the peripheral name). Non-STM32 boards have no
/// bus pin map here, so the `find_*_peripheral` lookups return `None` for them
/// (faithful to `findPinFunction` returning null for an unmapped board).
const STM32_BOARDS: &[&str] = &[
    "stm32f103",
    "stm32f401",
    "stm32f401cdu6",
    "stm32l476",
    "stm32h563",
];

/// SPI display/sensor devices addressed by their own emitter
/// (port of the TS `SPI_DEVICE_TYPES` set).
const SPI_DEVICE_TYPES: &[&str] = &["ili9341", "max31855", "ssd1680_tricolor_290"];

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

/// `board_io` kind for a part type — a faithful, TOTAL port of `COMPONENT_META`
/// (itself `CATALOG.boardIoKind` for every part). Returns the CATALOG kind
/// verbatim, including the `i2c_device`/`spi_device`/`uart_device` kinds:
/// [`emit_board_io_from_wires`] mirrors the TS emitter, which skips the legacy
/// I²C set, the [`SPI_DEVICE_TYPES`] set and the dedicated-emitter parts, then
/// emits whatever kind remains for any other wired part (e.g. a `pca9685` wired
/// straight to GPIO). `adc_input` remaps its peripheral to the pin's ADC
/// controller in that emitter.
///
/// Parts the CATALOG gives no `boardIoKind` return `None` — including the
/// direct-drive displays `seven-segment` and `tm1637-7seg`, which are owned by
/// dedicated emitters ([`emit_seven_segment`]) rather than the generic
/// `board_io` path.
fn board_io_kind(part_type: &str) -> Option<&'static str> {
    match part_type {
        "led" | "rgb-led" => Some("led"),
        "button" | "dht22" | "dip-switch" | "keypad" | "pir-sensor" | "rotary-encoder"
        | "slide-switch" | "ultrasonic" => Some("button"),
        "buzzer" | "l293d" | "servo" => Some("pwm_output"),
        "adxl345" | "bme280" | "fxos8700" | "ir" | "lcd1602" | "mlx90614" | "mpu6050"
        | "oled-ssd1306" | "pca9685" | "scd41" | "sgp41" | "sps30" | "veml7700" => {
            Some("i2c_device")
        }
        "74hc595"
        | "ili9341"
        | "led-matrix"
        | "max31855"
        | "neopixel"
        | "pcd8544"
        | "sn74hc165"
        | "ssd1680_tricolor_290"
        | "uc8151d_tricolor_290" => Some("spi_device"),
        "ldr" | "ntc-thermistor" | "potentiometer" => Some("adc_input"),
        "iolink-master" | "neo6m-gps" => Some("uart_device"),
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
/// pin. Ported from the STM32F103 base pin map (shared by F401/L476/H563); other
/// boards return `None` (faithful to `findPinFunction` returning null for an
/// unmapped board), leaving the device unbound.
fn find_i2c_peripheral(board: &str, pin_label: &str) -> Option<&'static str> {
    if !STM32_BOARDS.contains(&board) {
        return None;
    }
    match pin_label.to_ascii_uppercase().as_str() {
        "PB6" | "PB7" | "PB8" | "PB9" => Some("i2c1"),
        "PB10" | "PB11" => Some("i2c2"),
        _ => None,
    }
}

/// STM32 `findPinFunction(board, pin, 'spi')` — the SPI peripheral on the pin,
/// from the STM32F103 base pin map (SPI1 on PA4–PA7/PA15/PB3–PB5, SPI2 on
/// PB12–PB15, SPI3 on PC10–PC12). `None` for a non-STM32 board or an unmapped
/// pin.
fn find_spi_peripheral(board: &str, pin_label: &str) -> Option<&'static str> {
    if !STM32_BOARDS.contains(&board) {
        return None;
    }
    match pin_label.to_ascii_uppercase().as_str() {
        "PA4" | "PA5" | "PA6" | "PA7" | "PA15" | "PB3" | "PB4" | "PB5" => Some("spi1"),
        "PB12" | "PB13" | "PB14" | "PB15" => Some("spi2"),
        "PC10" | "PC11" | "PC12" => Some("spi3"),
        _ => None,
    }
}

/// STM32 `findPinFunction(board, pin, 'uart')` — the UART peripheral on the pin,
/// from the STM32F103 base pin map (USART1 on PA9/PA10, USART2 on PA2/PA3,
/// USART3 on PB10/PB11 & PC10/PC11). `None` for a non-STM32 board or an unmapped
/// pin.
fn find_uart_peripheral(board: &str, pin_label: &str) -> Option<&'static str> {
    if !STM32_BOARDS.contains(&board) {
        return None;
    }
    match pin_label.to_ascii_uppercase().as_str() {
        "PA9" | "PA10" => Some("uart1"),
        "PA2" | "PA3" => Some("uart2"),
        "PB10" | "PB11" | "PC10" | "PC11" => Some("uart3"),
        _ => None,
    }
}

/// STM32 `findPinFunction(board, pin, 'can')` → `(peripheral, role)` — the CAN
/// function on the pin, from the STM32F103 base pin map (bxCAN1 RX on PA11, TX
/// on PA12). `None` for a non-STM32 board or an unmapped pin.
fn find_can_function(board: &str, pin_label: &str) -> Option<(&'static str, &'static str)> {
    if !STM32_BOARDS.contains(&board) {
        return None;
    }
    match pin_label.to_ascii_uppercase().as_str() {
        "PA11" => Some(("bxcan1", "rx")),
        "PA12" => Some(("bxcan1", "tx")),
        _ => None,
    }
}

/// STM32 `findPinFunction(board, pin, 'adc')` — the ADC controller on the pin,
/// from the STM32F103 base pin map (ADC1 on PA0–PA7, PB0/PB1, PC0–PC5). The
/// emitter only needs the peripheral name; L476's differing ADC *channel*
/// numbers are irrelevant here. `None` for a non-STM32 board or an unmapped pin.
fn find_adc_peripheral(board: &str, pin_label: &str) -> Option<&'static str> {
    if !STM32_BOARDS.contains(&board) {
        return None;
    }
    match pin_label.to_ascii_uppercase().as_str() {
        "PA0" | "PA1" | "PA2" | "PA3" | "PA4" | "PA5" | "PA6" | "PA7" | "PB0" | "PB1" | "PC0"
        | "PC1" | "PC2" | "PC3" | "PC4" | "PC5" => Some("adc1"),
        _ => None,
    }
}

/// The SPI peripheral for a device's clock wire (port of `spiPeripheralForPart`):
/// the first of `CLK`/`SCK`/`DIN`/`MOSI` whose wired MCU pin has an SPI function.
fn spi_peripheral_for_part(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> Option<String> {
    for pin_id in ["CLK", "SCK", "DIN", "MOSI"] {
        if let Some(mcu_pin) = mcu_pin_for_part_pin(wires, mcu_id, part_id, pin_id) {
            if let Some(p) = find_spi_peripheral(board, mcu_pin) {
                return Some(p.to_string());
            }
        }
    }
    None
}

/// The UART peripheral for a device's RX/TX wire (port of `uartPeripheralForPart`).
fn uart_peripheral_for_part(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> Option<String> {
    for pin_id in ["RX", "TX"] {
        if let Some(mcu_pin) = mcu_pin_for_part_pin(wires, mcu_id, part_id, pin_id) {
            if let Some(p) = find_uart_peripheral(board, mcu_pin) {
                return Some(p.to_string());
            }
        }
    }
    None
}

// --- Net-based traversal (port of the wire-graph helpers in emitters.ts) -----

/// Every endpoint reachable from `start` over the undirected wire graph,
/// including `start` itself (port of `connectedEndpoints` — a BFS over both wire
/// directions). Endpoints are `(part, pin)` pairs; visitation order matches the
/// TS BFS (FIFO queue, adjacency lists built in wire order, `from→to` then
/// `to→from` per wire).
fn connected_endpoints(wires: &[Wire], start_part: &str, start_pin: &str) -> Vec<(String, String)> {
    let key = |part: &str, pin: &str| format!("{part}:{pin}");
    let mut by_key: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();
    let mut add_edge = |a: (&str, &str), b: (&str, &str)| {
        by_key
            .entry(key(a.0, a.1))
            .or_default()
            .push((b.0.to_string(), b.1.to_string()));
    };
    for w in wires {
        add_edge((&w.from_part, &w.from_pin), (&w.to_part, &w.to_pin));
        add_edge((&w.to_part, &w.to_pin), (&w.from_part, &w.from_pin));
    }

    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: std::collections::VecDeque<(String, String)> = std::collections::VecDeque::new();
    queue.push_back((start_part.to_string(), start_pin.to_string()));
    while let Some(endpoint) = queue.pop_front() {
        let k = key(&endpoint.0, &endpoint.1);
        if seen.contains(&k) {
            continue;
        }
        seen.insert(k.clone());
        if let Some(neighbours) = by_key.get(&k) {
            for n in neighbours {
                if !seen.contains(&key(&n.0, &n.1)) {
                    queue.push_back(n.clone());
                }
            }
        }
        out.push(endpoint);
    }
    out
}

/// MCU pins on the same net as `part_id:pin_id` (port of `mcuPinsOnNet`).
fn mcu_pins_on_net(wires: &[Wire], mcu_id: &str, part_id: &str, pin_id: &str) -> Vec<String> {
    connected_endpoints(wires, part_id, pin_id)
        .into_iter()
        .filter(|(part, _)| part == mcu_id)
        .map(|(_, pin)| pin)
        .collect()
}

/// The CAN peripheral bridged by a `can-transceiver` (port of
/// `canPeripheralForTransceiver`): TXD must reach exactly one CAN-`tx` peripheral
/// and RXD exactly one CAN-`rx`, and they must agree.
fn can_peripheral_for_transceiver(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> Option<String> {
    let peripheral_for = |pin_id: &str, role: &str| -> Option<String> {
        let mut matches: HashSet<String> = HashSet::new();
        for mcu_pin in mcu_pins_on_net(wires, mcu_id, part_id, pin_id) {
            if let Some((periph, r)) = find_can_function(board, &mcu_pin) {
                if r == role {
                    matches.insert(periph.to_string());
                }
            }
        }
        if matches.len() == 1 {
            matches.into_iter().next()
        } else {
            None
        }
    };
    let tx = peripheral_for("TXD", "tx");
    let rx = peripheral_for("RXD", "rx");
    match (tx, rx) {
        (Some(tx), Some(rx)) if tx == rx => Some(tx),
        _ => None,
    }
}

/// The CAN peripheral a diagnostic tool observes, via a wired `can-transceiver`
/// (port of `canPeripheralForDiagnosticTool`).
fn can_peripheral_for_diagnostic_tool(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    parts: &[CanonicalPart],
    part_id: &str,
) -> Option<String> {
    for pin_id in ["CAN_H", "CAN_L"] {
        for (ep_part, ep_pin) in connected_endpoints(wires, part_id, pin_id) {
            let Some(part) = parts.iter().find(|p| p.id == ep_part) else {
                continue;
            };
            if part.r#type != "can-transceiver" {
                continue;
            }
            if ep_pin != "CAN_H" && ep_pin != "CAN_L" {
                continue;
            }
            if let Some(p) = can_peripheral_for_transceiver(board, wires, mcu_id, &ep_part) {
                return Some(p);
            }
        }
    }
    None
}

/// The UART peripheral an `iolink-transceiver`'s TXD/RXD reach (port of
/// `uartPeripheralForIolinkTransceiver`): exactly one UART across both pins.
fn uart_peripheral_for_iolink_transceiver(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> Option<String> {
    let mut matches: HashSet<String> = HashSet::new();
    for pin_id in ["TXD", "RXD"] {
        for mcu_pin in mcu_pins_on_net(wires, mcu_id, part_id, pin_id) {
            if let Some(u) = find_uart_peripheral(board, &mcu_pin) {
                matches.insert(u.to_string());
            }
        }
    }
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

/// The UART peripheral for an `iolink-master` (port of
/// `uartPeripheralForIolinkMaster`): a direct RX/TX wire, else via a wired
/// `iolink-transceiver`'s CQ line.
fn uart_peripheral_for_iolink_master(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    parts: &[CanonicalPart],
    part_id: &str,
) -> Option<String> {
    if let Some(direct) = uart_peripheral_for_part(board, wires, mcu_id, part_id) {
        return Some(direct);
    }
    for pin_id in ["TX", "RX"] {
        for (ep_part, ep_pin) in connected_endpoints(wires, part_id, pin_id) {
            let Some(part) = parts.iter().find(|p| p.id == ep_part) else {
                continue;
            };
            if part.r#type != "iolink-transceiver" {
                continue;
            }
            if ep_pin != "CQ" {
                continue;
            }
            if let Some(u) = uart_peripheral_for_iolink_transceiver(board, wires, mcu_id, &ep_part)
            {
                return Some(u);
            }
        }
    }
    None
}

/// `String(attrs[key])`-style lookup: a string attr verbatim, any other JSON
/// value stringified (mirrors `attrsToStringMap` + `String(v)`). `None` when
/// absent.
fn attr_string(part: &CanonicalPart, key: &str) -> Option<String> {
    let v = part.attrs.as_ref()?.get(key)?;
    Some(match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    })
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

/// Format a numeric attr value the way the TS `${n}` template literal does:
/// integers without a decimal point, non-integers via Rust's default `f64`
/// formatting (adequate for the values these emitters produce).
fn format_js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Emit the `external_devices` fragment for an ultrasonic part (port of
/// `emitUltrasonic`). `distance_cm` defaults to 100; `cpu_hz` is 250000 on
/// stm32l476, else 80000000. Never emits `board_io` (ultrasonic is in the
/// wire-emitter skip set).
fn emit_ultrasonic(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part: &CanonicalPart,
) -> (Option<String>, Option<String>) {
    let trig = mcu_pin_for_part_pin(wires, mcu_id, &part.id, "TRIG");
    let echo = mcu_pin_for_part_pin(wires, mcu_id, &part.id, "ECHO");
    let (Some(trig), Some(echo)) = (trig, echo) else {
        return (None, None);
    };
    let distance_cm = attr_string(part, "distance")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|d| d.is_finite())
        .unwrap_or(100.0);
    let cpu_hz = if board == "stm32l476" {
        250_000
    } else {
        80_000_000
    };
    let ext = format!(
        "  - id: \"{}\"\n    type: \"hc-sr04\"\n    connection: \"gpio\"\n    config:\n      trig_pin: \"{trig}\"\n      echo_pin: \"{echo}\"\n      distance_cm: {}\n      cpu_hz: {cpu_hz}",
        part.id,
        format_js_number(distance_cm)
    );
    (Some(ext), None)
}

/// Emit the `external_devices` fragment for a direct-drive `seven-segment` part
/// (port of `emitSevenSegment`).
///
/// Nine pins go straight to GPIO. Unlike the TM1637, the pins need NOT share one
/// GPIO peripheral — the kit resolves each pin's output register independently,
/// so `connection` carries the COM pin's peripheral purely as the grouping key.
/// A–G and COM are required (any unwired one emits nothing); DP is optional and
/// is simply omitted from `config` when unwired.
fn emit_seven_segment(
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> (Option<String>, Option<String>) {
    const SEGS: [&str; 7] = ["A", "B", "C", "D", "E", "F", "G"];

    let mut seg_pins: Vec<&str> = Vec::with_capacity(SEGS.len());
    for seg in SEGS {
        let Some(pin) = mcu_pin_for_part_pin(wires, mcu_id, part_id, seg) else {
            return (None, None);
        };
        seg_pins.push(pin);
    }
    let Some(com_pin) = mcu_pin_for_part_pin(wires, mcu_id, part_id, "COM") else {
        return (None, None);
    };
    // `gpioForMcuPin` — the COM pin's GPIO peripheral is the grouping key.
    let Some((com_peripheral, _)) = parse_mcu_pin(com_pin) else {
        return (None, None);
    };
    let dp_pin = mcu_pin_for_part_pin(wires, mcu_id, part_id, "DP");

    let seg_config = SEGS
        .iter()
        .zip(&seg_pins)
        .map(|(seg, pin)| format!("      {}_pin: \"{pin}\"", seg.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join("\n");
    let dp_config = dp_pin.map_or_else(String::new, |p| format!("\n      dp_pin: \"{p}\""));

    let ext = format!(
        "  - id: \"{part_id}\"\n    type: \"seven-segment\"\n    connection: \"{com_peripheral}\"\n    config:\n{seg_config}{dp_config}\n      com_pin: \"{com_pin}\""
    );
    (Some(ext), None)
}

/// Emit the `external_devices` + `board_io` fragments for a pcd8544 part (port of
/// `emitPcd8544`).
fn emit_pcd8544(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> (Option<String>, Option<String>) {
    let connection = spi_peripheral_for_part(board, wires, mcu_id, part_id);
    let cs_pin = mcu_pin_for_part_pin(wires, mcu_id, part_id, "CE");
    let dc_pin = mcu_pin_for_part_pin(wires, mcu_id, part_id, "DC");
    let (Some(connection), Some(cs_pin), Some(dc_pin)) = (connection, cs_pin, dc_pin) else {
        return (None, None);
    };
    let ext = format!(
        "  - id: \"{part_id}\"\n    type: \"pcd8544\"\n    connection: \"{connection}\"\n    config:\n      cs_pin: \"{cs_pin}\"\n      dc_pin: \"{dc_pin}\""
    );
    let bio = parse_mcu_pin(cs_pin).map(|(_, pin)| {
        format!(
            "  - id: \"{part_id}\"\n    kind: \"spi_device\"\n    peripheral: \"{connection}\"\n    pin: {pin}\n    signal: \"input\"\n    active_high: true\n    device_type: \"pcd8544\""
        )
    });
    (Some(ext), bio)
}

/// Emit the `external_devices` fragment for a sn74hc165 part (port of
/// `emitSn74hc165`). `inputs` defaults to 165.
fn emit_sn74hc165(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part: &CanonicalPart,
) -> (Option<String>, Option<String>) {
    let connection = spi_peripheral_for_part(board, wires, mcu_id, &part.id);
    let cs_pin = mcu_pin_for_part_pin(wires, mcu_id, &part.id, "SH_LD");
    let (Some(connection), Some(cs_pin)) = (connection, cs_pin) else {
        return (None, None);
    };
    // Number.parseInt(attrs.inputs ?? '165', 10) || 165: a 0/NaN parse falls
    // back to 165.
    let inputs = attr_string(part, "inputs")
        .and_then(|s| parse_int_prefix(&s))
        .filter(|&n| n != 0)
        .unwrap_or(165);
    let ext = format!(
        "  - id: \"{}\"\n    type: \"sn74hc165\"\n    connection: \"{connection}\"\n    config:\n      cs_pin: \"{cs_pin}\"\n      inputs: {inputs}",
        part.id
    );
    (Some(ext), None)
}

/// Emit the `external_devices` fragment for an iolink-master part (port of
/// `emitIolinkMaster`).
fn emit_iolink_master(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    parts: &[CanonicalPart],
    part_id: &str,
) -> (Option<String>, Option<String>) {
    let Some(connection) = uart_peripheral_for_iolink_master(board, wires, mcu_id, parts, part_id)
    else {
        return (None, None);
    };
    let ext = format!(
        "  - id: \"{part_id}\"\n    type: \"iolink-master\"\n    connection: \"{connection}\"\n    config:\n      pd_in_len: 1\n      m_seq_type: 1\n      com: \"COM2\""
    );
    (Some(ext), None)
}

/// Emit the `external_devices` + `board_io` fragments for an SPI device from the
/// [`SPI_DEVICE_TYPES`] set (port of `emitSpiDevice`).
fn emit_spi_device(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
    part_type: &str,
) -> (Option<String>, Option<String>) {
    let connection = spi_peripheral_for_part(board, wires, mcu_id, part_id);
    let cs_pin = mcu_pin_for_part_pin(wires, mcu_id, part_id, "CS");
    let (Some(connection), Some(cs_pin)) = (connection, cs_pin) else {
        return (None, None);
    };
    let ext = format!(
        "  - id: \"{part_id}\"\n    type: \"{part_type}\"\n    connection: \"{connection}\"\n    config:\n      cs_pin: \"{cs_pin}\""
    );
    let bio = parse_mcu_pin(cs_pin).map(|(_, pin)| {
        format!(
            "  - id: \"{part_id}\"\n    kind: \"spi_device\"\n    peripheral: \"{connection}\"\n    pin: {pin}\n    signal: \"input\"\n    active_high: true\n    device_type: \"{part_type}\""
        )
    });
    (Some(ext), bio)
}

/// Emit the `external_devices` + `board_io` fragments for a neo6m-gps part (port
/// of `emitNeo6mGps`).
fn emit_neo6m_gps(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    part_id: &str,
) -> (Option<String>, Option<String>) {
    let Some(connection) = uart_peripheral_for_part(board, wires, mcu_id, part_id) else {
        return (None, None);
    };
    let ext = format!(
        "  - id: \"{part_id}\"\n    type: \"neo6m-gps\"\n    connection: \"{connection}\"\n    config: {{}}"
    );
    let bio = format!(
        "  - id: \"{part_id}\"\n    kind: \"uart_device\"\n    peripheral: \"{connection}\"\n    pin: 0\n    signal: \"input\"\n    active_high: true\n    device_type: \"neo6m-gps\""
    );
    (Some(ext), Some(bio))
}

/// Emit the `external_devices` fragment for an off-board CAN diagnostic tester
/// (port of `emitCanDiagnosticTool`). `request_id`/`request_data` default to
/// `0x7E0` / `03 22 F1 90`.
fn emit_can_diagnostic_tool(
    board: &str,
    wires: &[Wire],
    mcu_id: &str,
    parts: &[CanonicalPart],
    part: &CanonicalPart,
) -> (Option<String>, Option<String>) {
    let Some(connection) =
        can_peripheral_for_diagnostic_tool(board, wires, mcu_id, parts, &part.id)
    else {
        return (None, None);
    };
    let request_id = attr_string(part, "request_id").unwrap_or_else(|| "0x7E0".to_string());
    let request_data =
        attr_string(part, "request_data").unwrap_or_else(|| "03 22 F1 90".to_string());
    let ext = format!(
        "  - id: \"{}\"\n    type: \"can-diagnostic-tester\"\n    connection: \"{connection}\"\n    config:\n      request_id: \"{request_id}\"\n      request_data: \"{request_data}\"",
        part.id
    );
    (Some(ext), None)
}

/// JS `Number.parseInt(s, 10)`-style leading-integer parse: reads an optional
/// sign then the leading decimal digits, ignoring trailing junk. `None` when no
/// digits lead (JS `NaN`).
fn parse_int_prefix(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        neg = bytes[i] == b'-';
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    let n: i64 = s[start..i].parse().ok()?;
    Some(if neg { -n } else { n })
}

/// Emit `board_io` entries for point-to-point wires (port of
/// `emitBoardIoFromWires`). Skips parts owned by the dedicated emitters, the
/// legacy I²C set and the [`SPI_DEVICE_TYPES`] set, then emits whatever
/// `board_io_kind` remains for any other wired part. For an `adc_input` part the
/// peripheral is remapped to the pin's ADC controller when the pin has one.
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
        "seven-segment",
    ];

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
        let Some((mut peripheral, pin)) = parse_mcu_pin(mcu_pin) else {
            continue;
        };
        let signal = if kind == "button" || kind == "adc_input" {
            "input"
        } else {
            "output"
        };
        if kind == "adc_input" {
            if let Some(adc) = find_adc_peripheral(board, mcu_pin) {
                peripheral = adc.to_string();
            }
        }
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

    // -----------------------------------------------------------------------
    // Phase 2b fixtures: one per newly-ported emitter. Each FIXTURE_* is the
    // verbatim JSON from packages/board-config/test/fixtures/canonical/*.json and
    // each EXPECTED_* the verbatim oracle snapshot from that package's
    // test/__snapshots__/canonical.test.ts.snap.
    // -----------------------------------------------------------------------

    const FIXTURE_SPI_ILI9341: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "disp", "type": "ili9341" }
      ],
      "connections": [
        ["mcu:PA5", "disp:SCK"],
        ["mcu:PA7", "disp:MOSI"],
        ["mcu:PA4", "disp:CS"],
        ["mcu:3V3", "disp:VCC"],
        ["mcu:GND", "disp:GND"]
      ]
    }"#;

    const EXPECTED_SPI_ILI9341: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "disp"
    type: "ili9341"
    connection: "spi1"
    config:
      cs_pin: "PA4"
board_io:
  - id: "disp"
    kind: "spi_device"
    peripheral: "spi1"
    pin: 4
    signal: "input"
    active_high: true
    device_type: "ili9341"
"#;

    const FIXTURE_SPI_MAX31855: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "tc", "type": "max31855" }
      ],
      "connections": [
        ["mcu:PA5", "tc:SCK"],
        ["mcu:PA6", "tc:MISO"],
        ["mcu:PA4", "tc:CS"],
        ["mcu:3V3", "tc:VCC"],
        ["mcu:GND", "tc:GND"]
      ]
    }"#;

    const EXPECTED_SPI_MAX31855: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "tc"
    type: "max31855"
    connection: "spi1"
    config:
      cs_pin: "PA4"
board_io:
  - id: "tc"
    kind: "spi_device"
    peripheral: "spi1"
    pin: 4
    signal: "input"
    active_high: true
    device_type: "max31855"
"#;

    const FIXTURE_SPI_SSD1680: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "epd", "type": "ssd1680_tricolor_290" }
      ],
      "connections": [
        ["mcu:PA5", "epd:CLK"],
        ["mcu:PA7", "epd:DIN"],
        ["mcu:PA4", "epd:CS"],
        ["mcu:3V3", "epd:VCC"],
        ["mcu:GND", "epd:GND"]
      ]
    }"#;

    const EXPECTED_SPI_SSD1680: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "epd"
    type: "ssd1680_tricolor_290"
    connection: "spi1"
    config:
      cs_pin: "PA4"
board_io:
  - id: "epd"
    kind: "spi_device"
    peripheral: "spi1"
    pin: 4
    signal: "input"
    active_high: true
    device_type: "ssd1680_tricolor_290"
"#;

    const FIXTURE_ULTRASONIC: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "dist", "type": "ultrasonic" }
      ],
      "connections": [
        ["mcu:PA0", "dist:TRIG"],
        ["mcu:PA1", "dist:ECHO"],
        ["mcu:3V3", "dist:VCC"],
        ["mcu:GND", "dist:GND"]
      ]
    }"#;

    const EXPECTED_ULTRASONIC: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "dist"
    type: "hc-sr04"
    connection: "gpio"
    config:
      trig_pin: "PA0"
      echo_pin: "PA1"
      distance_cm: 100
      cpu_hz: 80000000
board_io:
  []
"#;

    const FIXTURE_PCD8544: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "lcd", "type": "pcd8544" }
      ],
      "connections": [
        ["mcu:PA5", "lcd:CLK"],
        ["mcu:PA7", "lcd:DIN"],
        ["mcu:PA4", "lcd:CE"],
        ["mcu:PA3", "lcd:DC"],
        ["mcu:3V3", "lcd:VCC"],
        ["mcu:GND", "lcd:GND"]
      ]
    }"#;

    const EXPECTED_PCD8544: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "lcd"
    type: "pcd8544"
    connection: "spi1"
    config:
      cs_pin: "PA4"
      dc_pin: "PA3"
board_io:
  - id: "lcd"
    kind: "spi_device"
    peripheral: "spi1"
    pin: 4
    signal: "input"
    active_high: true
    device_type: "pcd8544"
"#;

    const FIXTURE_SN74HC165: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "sr", "type": "sn74hc165" }
      ],
      "connections": [
        ["mcu:PA5", "sr:CLK"],
        ["mcu:PA6", "sr:QH"],
        ["mcu:PA4", "sr:SH_LD"],
        ["mcu:3V3", "sr:VCC"],
        ["mcu:GND", "sr:GND"]
      ]
    }"#;

    const EXPECTED_SN74HC165: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "sr"
    type: "sn74hc165"
    connection: "spi1"
    config:
      cs_pin: "PA4"
      inputs: 165
board_io:
  []
"#;

    const FIXTURE_IOLINK_MASTER: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "iol", "type": "iolink-master" }
      ],
      "connections": [
        ["mcu:PA3", "iol:RX"],
        ["mcu:PA2", "iol:TX"],
        ["mcu:3V3", "iol:VCC"],
        ["mcu:GND", "iol:GND"]
      ]
    }"#;

    const EXPECTED_IOLINK_MASTER: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "iol"
    type: "iolink-master"
    connection: "uart2"
    config:
      pd_in_len: 1
      m_seq_type: 1
      com: "COM2"
board_io:
  []
"#;

    const FIXTURE_NEO6M_GPS: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "gps", "type": "neo6m-gps" }
      ],
      "connections": [
        ["mcu:PA3", "gps:RX"],
        ["mcu:PA2", "gps:TX"],
        ["mcu:3V3", "gps:VCC"],
        ["mcu:GND", "gps:GND"]
      ]
    }"#;

    const EXPECTED_NEO6M_GPS: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "gps"
    type: "neo6m-gps"
    connection: "uart2"
    config: {}
board_io:
  - id: "gps"
    kind: "uart_device"
    peripheral: "uart2"
    pin: 0
    signal: "input"
    active_high: true
    device_type: "neo6m-gps"
"#;

    const FIXTURE_CAN_DIAGNOSTIC_TOOL: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "tc", "type": "can-transceiver" },
        { "id": "diag", "type": "can-diagnostic-tool" }
      ],
      "connections": [
        ["mcu:PA12", "tc:TXD"],
        ["mcu:PA11", "tc:RXD"],
        ["tc:CAN_H", "diag:CAN_H"],
        ["tc:CAN_L", "diag:CAN_L"],
        ["mcu:3V3", "tc:VCC"],
        ["mcu:GND", "tc:GND"]
      ]
    }"#;

    const EXPECTED_CAN_DIAGNOSTIC_TOOL: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "diag"
    type: "can-diagnostic-tester"
    connection: "bxcan1"
    config:
      request_id: "0x7E0"
      request_data: "03 22 F1 90"
board_io:
  []
"#;

    const FIXTURE_ADC_INPUT: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "pot", "type": "potentiometer" }
      ],
      "connections": [
        ["mcu:PA0", "pot:W"],
        ["mcu:3V3", "pot:1"],
        ["mcu:GND", "pot:2"]
      ]
    }"#;

    const EXPECTED_ADC_INPUT: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  []
board_io:
  - id: "pot"
    kind: "adc_input"
    peripheral: "adc1"
    pin: 0
    signal: "input"
    active_high: true
"#;

    const FIXTURE_SEVEN_SEGMENT: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "seg", "type": "seven-segment" }
      ],
      "connections": [
        ["mcu:PA0", "seg:A"],
        ["mcu:PA1", "seg:B"],
        ["mcu:PA4", "seg:C"],
        ["mcu:PA5", "seg:D"],
        ["mcu:PA6", "seg:E"],
        ["mcu:PA7", "seg:F"],
        ["mcu:PA8", "seg:G"],
        ["mcu:PA9", "seg:DP"],
        ["mcu:PB0", "seg:COM"]
      ]
    }"#;

    const EXPECTED_SEVEN_SEGMENT: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "seg"
    type: "seven-segment"
    connection: "gpiob"
    config:
      a_pin: "PA0"
      b_pin: "PA1"
      c_pin: "PA4"
      d_pin: "PA5"
      e_pin: "PA6"
      f_pin: "PA7"
      g_pin: "PA8"
      dp_pin: "PA9"
      com_pin: "PB0"
board_io:
  []
"#;

    /// DP is optional: the same panel with `DP` left unwired drops the
    /// `dp_pin:` line and emits everything else unchanged.
    const FIXTURE_SEVEN_SEGMENT_NO_DP: &str = r#"{
      "version": 1,
      "parts": [
        { "id": "mcu", "type": "nucleo-f401re" },
        { "id": "seg", "type": "seven-segment" }
      ],
      "connections": [
        ["mcu:PA0", "seg:A"],
        ["mcu:PA1", "seg:B"],
        ["mcu:PA4", "seg:C"],
        ["mcu:PA5", "seg:D"],
        ["mcu:PA6", "seg:E"],
        ["mcu:PA7", "seg:F"],
        ["mcu:PA8", "seg:G"],
        ["mcu:PB0", "seg:COM"]
      ]
    }"#;

    const EXPECTED_SEVEN_SEGMENT_NO_DP: &str = r#"name: "playground-board"
chip: "inline"
external_devices:
  - id: "seg"
    type: "seven-segment"
    connection: "gpiob"
    config:
      a_pin: "PA0"
      b_pin: "PA1"
      c_pin: "PA4"
      d_pin: "PA5"
      e_pin: "PA6"
      f_pin: "PA7"
      g_pin: "PA8"
      com_pin: "PB0"
board_io:
  []
"#;

    /// THE parity gate: for each fixture, the Rust `resolve()` must produce the
    /// exact `systemYaml` the TS oracle emits (byte-for-byte). Every fixture
    /// under packages/board-config/test/fixtures/canonical/*.json that has a
    /// board in CHIP_YAMLS (so the oracle can resolve it) is covered here.
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
            ("spi-ili9341", FIXTURE_SPI_ILI9341, EXPECTED_SPI_ILI9341),
            ("spi-max31855", FIXTURE_SPI_MAX31855, EXPECTED_SPI_MAX31855),
            ("spi-ssd1680", FIXTURE_SPI_SSD1680, EXPECTED_SPI_SSD1680),
            ("ultrasonic", FIXTURE_ULTRASONIC, EXPECTED_ULTRASONIC),
            ("pcd8544", FIXTURE_PCD8544, EXPECTED_PCD8544),
            ("sn74hc165", FIXTURE_SN74HC165, EXPECTED_SN74HC165),
            (
                "iolink-master",
                FIXTURE_IOLINK_MASTER,
                EXPECTED_IOLINK_MASTER,
            ),
            ("neo6m-gps", FIXTURE_NEO6M_GPS, EXPECTED_NEO6M_GPS),
            (
                "can-diagnostic-tool",
                FIXTURE_CAN_DIAGNOSTIC_TOOL,
                EXPECTED_CAN_DIAGNOSTIC_TOOL,
            ),
            ("adc-input", FIXTURE_ADC_INPUT, EXPECTED_ADC_INPUT),
            (
                "seven-segment",
                FIXTURE_SEVEN_SEGMENT,
                EXPECTED_SEVEN_SEGMENT,
            ),
            (
                "seven-segment-no-dp",
                FIXTURE_SEVEN_SEGMENT_NO_DP,
                EXPECTED_SEVEN_SEGMENT_NO_DP,
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

    /// Like the TS oracle, `resolve()` is total over part types: a passive/tool
    /// bridge part (here `can-transceiver`) contributes nothing on its own and is
    /// never an error, and a bus device whose clock pin has no SPI function on the
    /// board stays unbound (emits nothing) rather than erroring.
    #[test]
    fn resolve_ignores_unhandled_and_unbound_parts() {
        // A lone can-transceiver: no MCU-side CAN wiring, so it yields an empty
        // config — not an error.
        let json = r#"{
          "version": 1,
          "parts": [
            { "id": "mcu", "type": "nucleo-f401re" },
            { "id": "tc", "type": "can-transceiver" }
          ],
          "connections": [["mcu:PB9", "tc:TXD"]]
        }"#;
        let cfg = CanonicalConfig::from_json(json).unwrap();
        let yaml = cfg.resolve().expect("should resolve, not error");
        assert!(yaml.contains("external_devices:\n  []"), "got {yaml:?}");
        assert!(yaml.contains("board_io:\n  []"), "got {yaml:?}");

        // An ili9341 whose only wire is CS (no SPI clock pin) → spiPeripheralForPart
        // returns null → emitSpiDevice emits nothing. Faithful to the oracle.
        let json = r#"{
          "version": 1,
          "parts": [
            { "id": "mcu", "type": "nucleo-f401re" },
            { "id": "disp", "type": "ili9341" }
          ],
          "connections": [["mcu:PB9", "disp:CS"]]
        }"#;
        let cfg = CanonicalConfig::from_json(json).unwrap();
        let yaml = cfg.resolve().expect("should resolve, not error");
        assert!(yaml.contains("external_devices:\n  []"), "got {yaml:?}");
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
