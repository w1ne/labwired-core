// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Declarative IR for off-chip components (sensors, drivers, expanders).
//!
//! Sibling to [`crate::IrDevice`] (on-chip, memory-mapped). An `IrComponent`
//! describes a bus-attached device behaviorally: register file, pointer
//! semantics, wide-register read phasing, update rules, and observables.
//! A deterministic interpreter in the simulator core executes these specs;
//! see `labwired_core::peripherals::components::ir_component`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level declarative component spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrComponent {
    /// Part number, e.g. "PCA9685".
    pub name: String,
    /// Manufacturer, e.g. "NXP".
    #[serde(default)]
    pub vendor: Option<String>,
    /// Datasheet revision the behavior was derived from.
    #[serde(default)]
    pub datasheet: Option<String>,
    /// Execution kind. `wasm` is reserved by the design and rejected by
    /// validation until the WASM slice ships.
    #[serde(default)]
    pub kind: IrComponentKind,
    /// Bus interface. I2C (register-backed) or SPI (read-only response word).
    pub interface: IrComponentInterface,
    /// Backing register file. Required for I2C devices; omitted (defaults to
    /// empty) for read-only SPI devices, which describe their output via
    /// [`response`](IrComponent::response) instead.
    #[serde(default)]
    pub register_file: IrRegisterFile,
    /// Pointer (control-register) semantics. Optional: pointerless devices.
    #[serde(default)]
    pub pointer: Option<IrPointerRule>,
    /// Registers read as multi-byte values with a read phase (e.g. 16-bit BE).
    #[serde(default)]
    pub wide_registers: Vec<IrWideRegister>,
    /// State-update rules triggered by bus activity.
    #[serde(default)]
    pub updates: Vec<IrUpdateRule>,
    /// Named values derived from register state, readable by tests/run loop.
    #[serde(default)]
    pub observables: Vec<IrObservable>,
    /// Read-only SPI response framing. Required for — and only valid on — the
    /// `spi` interface. The device clocks this word out MSB-first on every
    /// CS-framed transaction (e.g. MAX31855, MAX6675, MCP3xxx).
    #[serde(default)]
    pub response: Option<IrResponse>,
}

/// Execution kind for a component spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IrComponentKind {
    /// Interpreted declaratively by the core (Slice 1).
    #[default]
    Declarative,
    /// Sandboxed WASM module (reserved; not yet supported).
    Wasm,
}

/// Bus interface binding.
///
/// Serializes/deserializes as `{ i2c: { default_address: N } }`.
///
/// # Serde strategy
///
/// All enum types in this module use **manual** `Serialize`/`Deserialize`
/// impls backed by plain-map helpers rather than `#[serde(untagged)]` or
/// `#[serde(tag = …)]`.  Under serde_yaml 0.9 the derive macros emit and
/// expect `!tag` YAML tag syntax, which is unreadable in hand-authored specs.
/// Manual impls keep the plain-map form (`{ i2c: { … } }`,
/// `{ when_field_set: { … } }`, etc.) and, critically, produce **strict,
/// targeted parse errors** for agent-facing diagnostics — an `#[serde(untagged)]`
/// approach would collapse every mismatch to "data did not match any variant"
/// and would silently accept typos such as `nevr` via a bare-string variant.
/// Do **not** replace these impls with `#[serde(untagged)]`.
#[derive(Debug, Clone, PartialEq)]
pub enum IrComponentInterface {
    /// I2C target device.
    I2c {
        /// 7-bit address when the manifest does not override it.
        default_address: u8,
    },
    /// Read-only SPI target device.
    Spi {
        /// Chip-select pin label when the manifest does not override it,
        /// e.g. "PA4".
        default_cs_pin: String,
    },
}

/// Serde helper for [`IrComponentInterface`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IrComponentInterfaceHelper {
    #[serde(skip_serializing_if = "Option::is_none")]
    i2c: Option<I2cFields>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spi: Option<SpiFields>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct I2cFields {
    default_address: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpiFields {
    default_cs_pin: String,
}

impl Serialize for IrComponentInterface {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let helper = match self {
            IrComponentInterface::I2c { default_address } => IrComponentInterfaceHelper {
                i2c: Some(I2cFields {
                    default_address: *default_address,
                }),
                spi: None,
            },
            IrComponentInterface::Spi { default_cs_pin } => IrComponentInterfaceHelper {
                i2c: None,
                spi: Some(SpiFields {
                    default_cs_pin: default_cs_pin.clone(),
                }),
            },
        };
        helper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for IrComponentInterface {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let helper = IrComponentInterfaceHelper::deserialize(deserializer)?;
        match (helper.i2c, helper.spi) {
            (Some(i2c), None) => Ok(IrComponentInterface::I2c {
                default_address: i2c.default_address,
            }),
            (None, Some(spi)) => Ok(IrComponentInterface::Spi {
                default_cs_pin: spi.default_cs_pin,
            }),
            (Some(_), Some(_)) => Err(serde::de::Error::custom(
                "interface declares both 'i2c' and 'spi'; choose exactly one",
            )),
            (None, None) => Err(serde::de::Error::missing_field("i2c' or 'spi")),
        }
    }
}

/// Flat byte-addressed register file.
///
/// Defaults to empty (`size: 0`) so read-only SPI specs may omit it; the
/// validator enforces a non-zero size for the I2C interface only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IrRegisterFile {
    /// Number of 8-bit registers (1..=65536).
    #[serde(default)]
    pub size: usize,
    /// Sparse non-zero reset values, keyed by register offset.
    #[serde(default)]
    pub reset: BTreeMap<u64, u8>,
}

/// Read-only SPI response word: a fixed-width big-endian value clocked out
/// MSB-first across [`bytes`](IrResponse::bytes) SPI transfers while CS is
/// asserted, composed from named bit-fields. Models read-only SPI sensors and
/// ADCs whose every CS-framed transaction returns the same status word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrResponse {
    /// Word width in bytes (1..=8), clocked MSB-first.
    pub bytes: u8,
    /// Bit-fields packed into the word.
    #[serde(default)]
    pub fields: Vec<IrResponseField>,
}

/// One bit-field packed into an [`IrResponse`] word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrResponseField {
    /// Field name; host stimulus addresses fields by name.
    pub name: String,
    /// LSB position of the field within the word.
    pub shift: u8,
    /// Field width in bits (1..=32).
    pub bits: u8,
    /// Whether the stored value is two's-complement signed. Affects only how a
    /// host-set value is masked into the word; the clocked-out bits are the
    /// low `bits` bits of the value either way.
    #[serde(default)]
    pub signed: bool,
    /// Power-on default value in field units (raw, pre-mask).
    #[serde(default)]
    pub reset: i64,
}

/// Write-pointer (control register) semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrPointerRule {
    /// The first byte written after START selects the register pointer.
    pub first_write_after_start_sets_pointer: bool,
    /// Mask applied to the pointer byte (e.g. TMP102 uses 0x03).
    #[serde(default = "default_pointer_mask")]
    pub pointer_mask: u8,
    /// When the pointer advances after a data byte.
    #[serde(default)]
    pub auto_increment: IrAutoIncrement,
}

fn default_pointer_mask() -> u8 {
    0xFF
}

/// Auto-increment policy for the register pointer.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum IrAutoIncrement {
    /// Pointer never advances automatically.
    #[default]
    Never,
    /// Pointer advances after every data byte.
    Always,
    /// Pointer advances when `regs[reg] & mask != 0` (e.g. PCA9685 MODE1.AI).
    WhenFieldSet {
        /// Register holding the enable field.
        reg: u64,
        /// Bit mask of the enable field.
        mask: u8,
    },
}

/// Serde helper for `when_field_set` variant of [`IrAutoIncrement`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhenFieldSetFields {
    reg: u64,
    mask: u8,
}

/// Serde helper map for [`IrAutoIncrement`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IrAutoIncrementMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    when_field_set: Option<WhenFieldSetFields>,
}

impl Serialize for IrAutoIncrement {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            IrAutoIncrement::Never => serializer.serialize_str("never"),
            IrAutoIncrement::Always => serializer.serialize_str("always"),
            IrAutoIncrement::WhenFieldSet { reg, mask } => {
                let m = IrAutoIncrementMap {
                    when_field_set: Some(WhenFieldSetFields {
                        reg: *reg,
                        mask: *mask,
                    }),
                };
                m.serialize(serializer)
            }
        }
    }
}

// IrAutoIncrement is the only enum here that routes through `serde_yaml::Value`
// because it uniquely accepts both a bare string ("never"/"always") and a map
// form ({ when_field_set: { … } }) — two structurally different YAML forms that
// cannot be unified by a single typed helper struct.
impl<'de> Deserialize<'de> for IrAutoIncrement {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::String(s) => match s.as_str() {
                "never" => Ok(IrAutoIncrement::Never),
                "always" => Ok(IrAutoIncrement::Always),
                other => Err(D::Error::unknown_variant(
                    other,
                    &["never", "always", "when_field_set"],
                )),
            },
            serde_yaml::Value::Mapping(_) => {
                let m: IrAutoIncrementMap =
                    serde_yaml::from_value(value).map_err(D::Error::custom)?;
                if let Some(f) = m.when_field_set {
                    Ok(IrAutoIncrement::WhenFieldSet {
                        reg: f.reg,
                        mask: f.mask,
                    })
                } else {
                    Err(D::Error::custom("unknown auto_increment variant"))
                }
            }
            _ => Err(D::Error::custom(
                "expected string or map for auto_increment",
            )),
        }
    }
}

/// A logical register wider than 8 bits, read big-endian over phased reads.
/// The read phase resets to MSB on START.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrWideRegister {
    /// Pointer value that selects this register.
    pub pointer: u8,
    /// Width in bits. Only 16 is supported in Slice 1.
    pub bits: u32,
    /// Initial value (big-endian semantics).
    #[serde(default)]
    pub reset: u16,
}

/// A deterministic state update fired by bus activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrUpdateRule {
    /// What fires the rule.
    pub trigger: IrUpdateTrigger,
    /// What happens.
    pub action: IrUpdateAction,
}

/// Update triggers.
#[derive(Debug, Clone, PartialEq)]
pub enum IrUpdateTrigger {
    /// A full read of the wide register selected by this pointer completed
    /// (both bytes of a 16-bit value were read).
    WideReadComplete {
        /// Pointer value of the wide register.
        pointer: u8,
    },
}

#[derive(Serialize, Deserialize)]
struct WideReadCompleteFields {
    pointer: u8,
}

#[derive(Serialize, Deserialize)]
struct IrUpdateTriggerMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    wide_read_complete: Option<WideReadCompleteFields>,
}

impl Serialize for IrUpdateTrigger {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            IrUpdateTrigger::WideReadComplete { pointer } => {
                let m = IrUpdateTriggerMap {
                    wide_read_complete: Some(WideReadCompleteFields { pointer: *pointer }),
                };
                m.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for IrUpdateTrigger {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let m = IrUpdateTriggerMap::deserialize(deserializer)?;
        if let Some(f) = m.wide_read_complete {
            Ok(IrUpdateTrigger::WideReadComplete { pointer: f.pointer })
        } else {
            Err(D::Error::custom("unknown IrUpdateTrigger variant"))
        }
    }
}

/// Update actions.
#[derive(Debug, Clone, PartialEq)]
pub enum IrUpdateAction {
    /// `value = value.wrapping_add(add); if value > max { value = reset }`
    /// applied to the triggering wide register (signed compare, as i16).
    AddWrap {
        /// Amount added per trigger (two's-complement i16).
        add: i16,
        /// Exclusive upper bound (i16 compare).
        max: i16,
        /// Value assigned on wrap.
        reset: i16,
    },
}

#[derive(Serialize, Deserialize)]
struct AddWrapFields {
    add: i16,
    max: i16,
    reset: i16,
}

#[derive(Serialize, Deserialize)]
struct IrUpdateActionMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    add_wrap: Option<AddWrapFields>,
}

impl Serialize for IrUpdateAction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            IrUpdateAction::AddWrap { add, max, reset } => {
                let m = IrUpdateActionMap {
                    add_wrap: Some(AddWrapFields {
                        add: *add,
                        max: *max,
                        reset: *reset,
                    }),
                };
                m.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for IrUpdateAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let m = IrUpdateActionMap::deserialize(deserializer)?;
        if let Some(f) = m.add_wrap {
            Ok(IrUpdateAction::AddWrap {
                add: f.add,
                max: f.max,
                reset: f.reset,
            })
        } else {
            Err(D::Error::custom("unknown IrUpdateAction variant"))
        }
    }
}

/// A named, channel-indexed value derived from the register file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrObservable {
    /// Observable name, e.g. "servo_angle".
    pub name: String,
    /// Number of channels (1 for scalar observables).
    pub channels: u8,
    /// Register offset of channel 0's block.
    pub base: u64,
    /// Offset between consecutive channel blocks.
    pub stride: u64,
    /// How the raw value is composed from the channel block.
    pub value: IrObservableValue,
    /// Optional mapping from raw value to engineering units.
    #[serde(default)]
    pub map: Option<IrObservableMap>,
}

/// Raw-value composition.
#[derive(Debug, Clone, PartialEq)]
pub enum IrObservableValue {
    /// `((regs[base+hi_rel] & hi_mask) << 8) | regs[base+lo_rel]`
    U12Compose {
        /// Low-byte offset within the channel block.
        lo_rel: u64,
        /// High-byte offset within the channel block.
        hi_rel: u64,
        /// Mask applied to the high byte before shifting.
        hi_mask: u8,
    },
}

#[derive(Serialize, Deserialize)]
struct U12ComposeFields {
    lo_rel: u64,
    hi_rel: u64,
    hi_mask: u8,
}

#[derive(Serialize, Deserialize)]
struct IrObservableValueMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    u12_compose: Option<U12ComposeFields>,
}

impl Serialize for IrObservableValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            IrObservableValue::U12Compose {
                lo_rel,
                hi_rel,
                hi_mask,
            } => {
                let m = IrObservableValueMap {
                    u12_compose: Some(U12ComposeFields {
                        lo_rel: *lo_rel,
                        hi_rel: *hi_rel,
                        hi_mask: *hi_mask,
                    }),
                };
                m.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for IrObservableValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let m = IrObservableValueMap::deserialize(deserializer)?;
        if let Some(f) = m.u12_compose {
            Ok(IrObservableValue::U12Compose {
                lo_rel: f.lo_rel,
                hi_rel: f.hi_rel,
                hi_mask: f.hi_mask,
            })
        } else {
            Err(D::Error::custom("unknown IrObservableValue variant"))
        }
    }
}

/// Raw → engineering-units mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrObservableMap {
    /// `eng = clamp(raw * scale + offset)`.
    pub linear: IrLinearMap,
    /// Return `None` while the raw value is exactly 0 (channel never written).
    #[serde(default)]
    pub none_when_raw_zero: bool,
}

/// Linear map coefficients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrLinearMap {
    /// Multiplier.
    pub scale: f32,
    /// Added after scaling.
    pub offset: f32,
    /// Optional inclusive clamp range.
    #[serde(default)]
    pub clamp: Option<(f32, f32)>,
}

/// A validation finding. Mirrors the diagram-diagnostic shape used by the
/// MCP surface: stable code + human message + suggested fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrComponentDiag {
    /// Stable machine-readable code, e.g. "ICOMP_RESET_OUT_OF_RANGE".
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Suggested fix.
    pub hint: String,
}

impl IrComponentDiag {
    fn new(code: &str, message: String, hint: &str) -> Self {
        Self {
            code: code.into(),
            message,
            hint: hint.into(),
        }
    }
}

impl IrComponent {
    /// Validate the spec. Empty result means executable. Every condition the
    /// interpreter cannot execute deterministically is rejected here, never
    /// at run time.
    pub fn validate(&self) -> Vec<IrComponentDiag> {
        let mut out = Vec::new();
        if self.kind == IrComponentKind::Wasm {
            out.push(IrComponentDiag::new(
                "ICOMP_WASM_UNSUPPORTED",
                "kind: wasm is reserved and not yet executable".into(),
                "Use kind: declarative",
            ));
        }
        // SPI (read-only response word) is validated on its own path; the
        // register-file / pointer / wide / observable checks below are
        // I2C-only and would mis-fire on the empty register file of an SPI
        // spec (e.g. size 0).
        if let IrComponentInterface::Spi { default_cs_pin } = &self.interface {
            self.validate_spi(default_cs_pin, &mut out);
            return out;
        }
        let size = self.register_file.size as u64;
        if self.register_file.size == 0 || self.register_file.size > 65536 {
            out.push(IrComponentDiag::new(
                "ICOMP_REGFILE_SIZE",
                format!(
                    "register_file.size {} outside 1..=65536",
                    self.register_file.size
                ),
                "Choose the device's real register-file size (often 256)",
            ));
        }
        for &off in self.register_file.reset.keys() {
            if off >= size {
                out.push(IrComponentDiag::new(
                    "ICOMP_RESET_OUT_OF_RANGE",
                    format!(
                        "reset value at offset {off:#x} outside register file (size {size:#x})"
                    ),
                    "Remove the entry or grow register_file.size",
                ));
            }
        }
        if let Some(p) = &self.pointer {
            if let IrAutoIncrement::WhenFieldSet { reg, .. } = p.auto_increment {
                if reg >= size {
                    out.push(IrComponentDiag::new(
                        "ICOMP_AI_REG_OUT_OF_RANGE",
                        format!("auto_increment enable register {reg:#x} outside register file"),
                        "Point at a register inside the file",
                    ));
                }
            }
            // The masked pointer can select offsets 0..=pointer_mask; if
            // pointer_mask >= size then a datasheet-faithful device cannot exist
            // entirely within this register file (the interpreter would have to
            // alias/wrap silently).
            if p.pointer_mask as u64 >= size {
                out.push(IrComponentDiag::new(
                    "ICOMP_POINTER_MASK_RANGE",
                    format!(
                        "pointer_mask {:#x} can select offsets outside the register file (size {size:#x})",
                        p.pointer_mask
                    ),
                    "Reduce pointer_mask or grow register_file.size so every selectable pointer is in range",
                ));
            }
        }
        for w in &self.wide_registers {
            if w.bits != 16 {
                out.push(IrComponentDiag::new(
                    "ICOMP_WIDE_BITS_UNSUPPORTED",
                    format!(
                        "wide register at pointer {:#x} has bits={}, only 16 supported",
                        w.pointer, w.bits
                    ),
                    "Use bits: 16",
                ));
            }
        }
        for u in &self.updates {
            let IrUpdateTrigger::WideReadComplete { pointer } = u.trigger;
            if !self.wide_registers.iter().any(|w| w.pointer == pointer) {
                out.push(IrComponentDiag::new(
                    "ICOMP_UPDATE_DANGLING",
                    format!(
                        "update trigger references undeclared wide register pointer {pointer:#x}"
                    ),
                    "Declare the wide register or fix the pointer",
                ));
            }
        }
        for o in &self.observables {
            let IrObservableValue::U12Compose { lo_rel, hi_rel, .. } = o.value;
            let span = lo_rel.max(hi_rel);
            // channels.max(1): treat a zero-channel observable like one channel
            // so the out-of-range check is still enforced on the base block.
            let last = o
                .base
                .checked_add(o.stride.saturating_mul(o.channels.max(1) as u64 - 1))
                .and_then(|v| v.checked_add(span))
                .unwrap_or(u64::MAX);
            if last >= size {
                out.push(IrComponentDiag::new(
                    "ICOMP_OBS_OUT_OF_RANGE",
                    format!(
                        "observable '{}' channel block ends at {last:#x}, outside register file (size {size:#x})",
                        o.name
                    ),
                    "Reduce channels/base/stride or grow register_file.size",
                ));
            }
        }
        if self.response.is_some() {
            out.push(IrComponentDiag::new(
                "ICOMP_RESPONSE_ON_I2C",
                "response: is only valid on the spi interface".into(),
                "Remove the response block, or switch interface to spi",
            ));
        }
        out
    }

    /// Validate a read-only SPI spec. The `response` block is required; the
    /// register-file / pointer / wide / observable / update sections are
    /// I2C-only and rejected here so a spec cannot silently mix the two
    /// execution models.
    fn validate_spi(&self, default_cs_pin: &str, out: &mut Vec<IrComponentDiag>) {
        if default_cs_pin.trim().is_empty() {
            out.push(IrComponentDiag::new(
                "ICOMP_SPI_CS_EMPTY",
                "spi.default_cs_pin is empty".into(),
                "Set the chip-select pin label, e.g. default_cs_pin: PA4",
            ));
        }
        let Some(resp) = &self.response else {
            out.push(IrComponentDiag::new(
                "ICOMP_SPI_NO_RESPONSE",
                "spi interface requires a response: block".into(),
                "Add response: { bytes: N, fields: [...] }",
            ));
            return;
        };
        if resp.bytes == 0 || resp.bytes > 8 {
            out.push(IrComponentDiag::new(
                "ICOMP_SPI_RESPONSE_BYTES",
                format!("response.bytes {} outside 1..=8", resp.bytes),
                "Use the device's word width in bytes (MAX31855 = 4)",
            ));
        }
        let word_bits = (resp.bytes as u32) * 8;
        for f in &resp.fields {
            if f.bits == 0 || f.bits > 32 {
                out.push(IrComponentDiag::new(
                    "ICOMP_SPI_FIELD_BITS",
                    format!(
                        "response field '{}' has bits={} outside 1..=32",
                        f.name, f.bits
                    ),
                    "Use the field's real width in bits",
                ));
                continue;
            }
            if f.shift as u32 + f.bits as u32 > word_bits {
                out.push(IrComponentDiag::new(
                    "ICOMP_SPI_FIELD_RANGE",
                    format!(
                        "response field '{}' (shift {} + bits {}) overflows the {word_bits}-bit word",
                        f.name, f.shift, f.bits
                    ),
                    "Reduce shift/bits or grow response.bytes",
                ));
            }
        }
        // Reject I2C-only sections on an SPI device.
        let mut reject = |present: bool, code: &str, what: &str| {
            if present {
                out.push(IrComponentDiag::new(
                    code,
                    format!("{what} is not valid on the spi interface"),
                    "Remove it; SPI devices describe output via response:",
                ));
            }
        };
        reject(
            self.register_file.size != 0 || !self.register_file.reset.is_empty(),
            "ICOMP_SPI_HAS_REGFILE",
            "register_file",
        );
        reject(self.pointer.is_some(), "ICOMP_SPI_HAS_POINTER", "pointer");
        reject(
            !self.wide_registers.is_empty(),
            "ICOMP_SPI_HAS_WIDE",
            "wide_registers",
        );
        reject(
            !self.observables.is_empty(),
            "ICOMP_SPI_HAS_OBSERVABLES",
            "observables",
        );
        reject(!self.updates.is_empty(), "ICOMP_SPI_HAS_UPDATES", "updates");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pca9685_shaped_spec_round_trips_yaml() {
        let yaml = r#"
name: PCA9685
vendor: NXP
datasheet: "PCA9685 Rev. 4"
kind: declarative
interface:
  i2c:
    default_address: 0x40
register_file:
  size: 256
  reset:
    0x00: 0x11
pointer:
  first_write_after_start_sets_pointer: true
  auto_increment:
    when_field_set: { reg: 0x00, mask: 0x20 }
observables:
  - name: servo_angle
    channels: 16
    base: 0x06
    stride: 4
    value:
      u12_compose: { lo_rel: 2, hi_rel: 3, hi_mask: 0x0F }
    map:
      linear: { scale: 0.46258224, offset: -47.368423, clamp: [0.0, 180.0] }
      none_when_raw_zero: true
"#;
        let spec: IrComponent = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(spec.name, "PCA9685");
        let IrComponentInterface::I2c { default_address } = &spec.interface else {
            panic!("expected i2c interface, got {:?}", spec.interface);
        };
        assert_eq!(*default_address, 0x40);
        assert_eq!(spec.register_file.size, 256);
        assert_eq!(spec.register_file.reset.get(&0x00), Some(&0x11));
        let obs = &spec.observables[0];
        assert_eq!(obs.channels, 16);
        // Round-trip: serialize and re-parse; assert whole-struct equality to
        // cover all enum-carrying fields (interface, pointer.auto_increment,
        // observables[0].value).  IrLinearMap contains f32 fields — if YAML
        // serialization loses precision the assertion below will fail; fall
        // back to the field-level assertions in that case.
        let re: IrComponent = serde_yaml::from_str(&serde_yaml::to_string(&spec).unwrap()).unwrap();
        assert_eq!(re, spec);
    }

    fn minimal_spec() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: X
kind: declarative
interface: { i2c: { default_address: 0x40 } }
register_file: { size: 256 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn valid_minimal_spec_has_no_diagnostics() {
        assert!(minimal_spec().validate().is_empty());
    }

    #[test]
    fn wasm_kind_is_rejected_until_supported() {
        let mut s = minimal_spec();
        s.kind = IrComponentKind::Wasm;
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_WASM_UNSUPPORTED"),
            "{d:?}"
        );
    }

    #[test]
    fn reset_offset_out_of_range_is_diagnosed() {
        let mut s = minimal_spec();
        s.register_file.reset.insert(300, 1); // size is 256
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_RESET_OUT_OF_RANGE"),
            "{d:?}"
        );
    }

    #[test]
    fn observable_block_must_fit_register_file() {
        let mut s = minimal_spec();
        s.observables.push(IrObservable {
            name: "x".into(),
            channels: 16,
            base: 0xF8, // 0xF8 + 15*4 + 3 > 255
            stride: 4,
            value: IrObservableValue::U12Compose {
                lo_rel: 2,
                hi_rel: 3,
                hi_mask: 0x0F,
            },
            map: None,
        });
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_OBS_OUT_OF_RANGE"),
            "{d:?}"
        );
    }

    #[test]
    fn wide_register_only_16_bits() {
        let mut s = minimal_spec();
        s.wide_registers.push(IrWideRegister {
            pointer: 0,
            bits: 24,
            reset: 0,
        });
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_WIDE_BITS_UNSUPPORTED"),
            "{d:?}"
        );
    }

    #[test]
    fn update_trigger_must_reference_declared_wide_register() {
        let mut s = minimal_spec();
        s.updates.push(IrUpdateRule {
            trigger: IrUpdateTrigger::WideReadComplete { pointer: 7 },
            action: IrUpdateAction::AddWrap {
                add: 1,
                max: 10,
                reset: 0,
            },
        });
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_UPDATE_DANGLING"), "{d:?}");
    }

    // --- Fix 1: register_file.size boundary ---

    #[test]
    fn regfile_size_zero_is_diagnosed() {
        let mut s = minimal_spec();
        s.register_file.size = 0;
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_REGFILE_SIZE"), "{d:?}");
    }

    #[test]
    fn regfile_size_too_large_is_diagnosed() {
        let mut s = minimal_spec();
        s.register_file.size = 70000;
        let d = s.validate();
        assert!(d.iter().any(|d| d.code == "ICOMP_REGFILE_SIZE"), "{d:?}");
    }

    // --- Fix 1: overflow-safe observable bounds (no panic on huge values) ---

    #[test]
    fn observable_huge_base_does_not_panic_and_is_diagnosed() {
        let mut s = minimal_spec();
        s.observables.push(IrObservable {
            name: "huge".into(),
            channels: 1,
            base: u64::MAX - 1,
            stride: 0,
            value: IrObservableValue::U12Compose {
                lo_rel: 0,
                hi_rel: 1,
                hi_mask: 0xFF,
            },
            map: None,
        });
        // Must not panic; must emit ICOMP_OBS_OUT_OF_RANGE.
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_OBS_OUT_OF_RANGE"),
            "{d:?}"
        );
    }

    // --- Fix 2: pointer_mask range validation ---

    #[test]
    fn pointer_mask_wider_than_register_file_is_diagnosed() {
        // Default pointer_mask = 0xFF; size = 16 → 0xFF >= 16 → error.
        let mut s = minimal_spec();
        s.register_file.size = 16;
        s.pointer = Some(IrPointerRule {
            first_write_after_start_sets_pointer: true,
            pointer_mask: 0xFF, // default
            auto_increment: IrAutoIncrement::Never,
        });
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_POINTER_MASK_RANGE"),
            "{d:?}"
        );
    }

    #[test]
    fn tmp102_shaped_spec_pointer_mask_is_valid() {
        // TMP102: size 4, pointer_mask 0x03 → mask (3) < size (4): no error.
        let mut s = minimal_spec();
        s.register_file.size = 4;
        s.pointer = Some(IrPointerRule {
            first_write_after_start_sets_pointer: true,
            pointer_mask: 0x03,
            auto_increment: IrAutoIncrement::Never,
        });
        let d = s.validate();
        assert!(
            !d.iter().any(|d| d.code == "ICOMP_POINTER_MASK_RANGE"),
            "TMP102-shaped spec should not trigger ICOMP_POINTER_MASK_RANGE: {d:?}"
        );
    }

    // --- Multiple diagnostics accumulate in a single validate() pass ---

    #[test]
    fn multiple_diagnostics_accumulate() {
        // wasm kind + a reset offset that is out of range → at least 2 entries.
        let mut s = minimal_spec();
        s.kind = IrComponentKind::Wasm;
        s.register_file.reset.insert(300, 1); // 300 >= 256
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_WASM_UNSUPPORTED"),
            "expected ICOMP_WASM_UNSUPPORTED in {d:?}"
        );
        assert!(
            d.iter().any(|d| d.code == "ICOMP_RESET_OUT_OF_RANGE"),
            "expected ICOMP_RESET_OUT_OF_RANGE in {d:?}"
        );
        assert!(d.len() >= 2, "expected at least 2 diagnostics, got: {d:?}");
    }

    // ── SPI interface (read-only response word) ───────────────────────────────

    fn spi_spec() -> IrComponent {
        serde_yaml::from_str(
            r#"
name: MAX31855
interface: { spi: { default_cs_pin: PA4 } }
response:
  bytes: 4
  fields:
    - { name: tc_temp, shift: 18, bits: 14, signed: true, reset: 100 }
    - { name: fault, shift: 16, bits: 1, reset: 0 }
    - { name: internal_temp, shift: 4, bits: 12, signed: true, reset: 352 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn valid_spi_spec_has_no_diagnostics() {
        assert!(
            spi_spec().validate().is_empty(),
            "{:?}",
            spi_spec().validate()
        );
    }

    #[test]
    fn spi_interface_round_trips_yaml() {
        let spec = spi_spec();
        let IrComponentInterface::Spi { default_cs_pin } = &spec.interface else {
            panic!("expected spi interface");
        };
        assert_eq!(default_cs_pin, "PA4");
        let re: IrComponent = serde_yaml::from_str(&serde_yaml::to_string(&spec).unwrap()).unwrap();
        assert_eq!(re, spec, "spi spec did not round-trip");
    }

    #[test]
    fn interface_with_both_i2c_and_spi_is_rejected() {
        let err = serde_yaml::from_str::<IrComponent>(
            r#"
name: ambiguous
interface: { i2c: { default_address: 0x40 }, spi: { default_cs_pin: PA4 } }
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("both"), "got: {err}");
    }

    #[test]
    fn spi_without_response_is_diagnosed() {
        let mut s = spi_spec();
        s.response = None;
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_SPI_NO_RESPONSE"),
            "expected ICOMP_SPI_NO_RESPONSE in {d:?}"
        );
    }

    #[test]
    fn spi_field_overflowing_word_is_diagnosed() {
        let mut s = spi_spec();
        // shift 30 + bits 14 = 44 > 32-bit word.
        s.response.as_mut().unwrap().fields[0].shift = 30;
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_SPI_FIELD_RANGE"),
            "expected ICOMP_SPI_FIELD_RANGE in {d:?}"
        );
    }

    #[test]
    fn spi_response_bytes_out_of_range_is_diagnosed() {
        let mut s = spi_spec();
        s.response.as_mut().unwrap().bytes = 9;
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_SPI_RESPONSE_BYTES"),
            "expected ICOMP_SPI_RESPONSE_BYTES in {d:?}"
        );
    }

    #[test]
    fn spi_with_i2c_only_sections_is_diagnosed() {
        let mut s = spi_spec();
        s.register_file.size = 8; // I2C-only section on an SPI device
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_SPI_HAS_REGFILE"),
            "expected ICOMP_SPI_HAS_REGFILE in {d:?}"
        );
    }

    #[test]
    fn response_on_i2c_interface_is_diagnosed() {
        let mut s = minimal_spec();
        s.response = Some(IrResponse {
            bytes: 4,
            fields: vec![],
        });
        let d = s.validate();
        assert!(
            d.iter().any(|d| d.code == "ICOMP_RESPONSE_ON_I2C"),
            "expected ICOMP_RESPONSE_ON_I2C in {d:?}"
        );
    }
}
