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
    /// Bus interface. I2C only in Slice 1.
    pub interface: IrComponentInterface,
    /// Backing register file.
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
}

/// Serde helper for [`IrComponentInterface`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IrComponentInterfaceHelper {
    #[serde(skip_serializing_if = "Option::is_none")]
    i2c: Option<I2cFields>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct I2cFields {
    default_address: u8,
}

impl Serialize for IrComponentInterface {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            IrComponentInterface::I2c { default_address } => {
                let helper = IrComponentInterfaceHelper {
                    i2c: Some(I2cFields {
                        default_address: *default_address,
                    }),
                };
                helper.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for IrComponentInterface {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let helper = IrComponentInterfaceHelper::deserialize(deserializer)?;
        if let Some(i2c) = helper.i2c {
            Ok(IrComponentInterface::I2c {
                default_address: i2c.default_address,
            })
        } else {
            Err(serde::de::Error::missing_field("i2c"))
        }
    }
}

/// Flat byte-addressed register file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrRegisterFile {
    /// Number of 8-bit registers (1..=65536).
    pub size: usize,
    /// Sparse non-zero reset values, keyed by register offset.
    #[serde(default)]
    pub reset: BTreeMap<u64, u8>,
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
      linear: { scale: 0.46291754, offset: -47.368423, clamp: [0.0, 180.0] }
      none_when_raw_zero: true
"#;
        let spec: IrComponent = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(spec.name, "PCA9685");
        let IrComponentInterface::I2c { default_address } = &spec.interface;
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
}
