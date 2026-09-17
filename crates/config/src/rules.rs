// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Tier-2 part schema** — states, variables, FIFOs, output pins, framing, and
//! the event/condition/action rules that tie them together.
//!
//! Everything here is an OPTIONAL field on the existing `behavior:` block, so
//! every shipped `labwired.part/v1` document keeps parsing byte for byte and
//! the transcript ratchet (`tests/declarative_device_byte_parity.rs`) is
//! unmoved. A part that declares none of it is a Tier-1 part and runs exactly
//! the code it ran before.
//!
//! What Tier 2 buys
//! ================
//! Tier 1 makes a register map data. That covers a sensor whose whole interface
//! is "read this word". It does not cover the majority: a part with an internal
//! mode, a command sequence, a sample FIFO, or an interrupt line. Those need
//! four things a register map cannot express — a **state**, a **variable**, a
//! **queue**, and a **pin the device drives**. This module is those four plus
//! the rule that moves between them.
//!
//! The vocabulary, deliberately small
//! ==================================
//! * [`Event`] — what happened on the wire or the clock.
//! * `when:` — an integer guard ([`crate::expr`]), absent ⇒ always.
//! * [`Action`] — what the device does about it.
//!
//! Rules fire **in declaration order**, and each `do:` list runs to completion.
//! A rule may [`Action::Goto`] a new state, and rules LATER in the same event
//! then evaluate against the NEW state. That is a choice, not an accident: it
//! makes a two-step sequence (`goto: armed` then, guarded on `state == armed`,
//! the thing an armed part does) expressible in one event without a second
//! event to carry it. The cost is that rule order matters inside an event; the
//! alternative — snapshotting the state for the whole event — makes the common
//! sequence impossible to write and was rejected for that reason.
//!
//! Expressions are integers only and are parsed ONCE, at load, by
//! [`compile_rules`]. A malformed expression is a load error that names the
//! rule index and the offending token, so a bad part document fails in manifest
//! preflight rather than at the first bus transaction.

use std::collections::BTreeMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::expr::{Expr, ExprError};

// ─── timers ────────────────────────────────────────────────────────────────
//
// There is no timer type here. `behavior.timers:` is
// [`crate::DeviceTimer`] — Phase B's, which already carries `name`,
// `period_us`/`after_us`, `start`, `start_on_write` and `on_fire`. A Tier-2
// rule listens for `on: { timer: NAME }` on the SAME timer that fires those
// register actions, so a part has exactly one clock vocabulary and a rule and
// an `on_fire` cannot disagree about when it ticked.

// ─── FIFOs ─────────────────────────────────────────────────────────────────

/// A sample queue the device fills on its own schedule and firmware drains.
///
/// This is the shape a FIFO sensor (BMI270, MAX30102) actually has, and the one
/// a register map cannot fake: the depth and the overflow policy are the whole
/// point. A model that always hands back the newest sample passes firmware that
/// never drains fast enough — precisely the CPU-starvation bug worth simulating.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct FifoSpec {
    /// Name a `push:` / `pop:` action and `fifo_len()` refer to.
    pub name: String,
    /// Bytes per entry. An entry read out of a register is truncated to this.
    #[serde(default = "one_u8")]
    pub width_bytes: u8,
    /// How many entries fit before [`overflow`](Self::overflow) decides.
    pub depth: usize,
    /// SimInput channel whose current (encoded) value a `push:` with no
    /// explicit `value:` enqueues. Absent ⇒ every push must carry a `value:`.
    #[serde(default)]
    pub source: Option<String>,
    /// What a push into a full FIFO does.
    #[serde(default)]
    pub overflow: FifoOverflow,
}

fn one_u8() -> u8 {
    1
}

/// What happens when a full FIFO is pushed.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FifoOverflow {
    /// Discard the OLDEST entry to make room — a circular buffer.
    #[default]
    DropOldest,
    /// Discard the incoming entry — a part that stops sampling when full.
    DropNewest,
}

// ─── framing ───────────────────────────────────────────────────────────────

/// Framing for a part whose unit of work is a *message*, not a register: a
/// command shell over a stream. A `frame` event fires when the declared length
/// is reached, and always at the end of a transaction (`stop` / `cs_release`)
/// so a short frame is still delivered rather than silently swallowed.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct FrameSpec {
    /// Fixed frame length in bytes. Absent ⇒ the frame ends at the transaction
    /// boundary only.
    #[serde(default)]
    pub length: Option<u16>,
    /// Whether the first byte of a frame is an opcode (recorded in `var(opcode)`
    /// so a rule can switch on it).
    #[serde(default)]
    pub opcode_byte: bool,
    /// CRC-8 parameters, when the frame carries a trailing checksum.
    #[serde(default)]
    pub crc: Option<crate::Crc8Spec>,
}

// ─── named bit-fields ──────────────────────────────────────────────────────

/// A NAMED bit-field of a register, so `set: INT_STATUS.DATA_RDY` and
/// `field(INT_STATUS.DATA_RDY)` mean something.
///
/// Distinct from [`crate::FieldSpec`], which is a *sourced* sub-word used to
/// ASSEMBLE a composite measurement register (the MAX31855 frame). This one
/// names bits that already exist in the stored word. Keeping them apart is why
/// neither had to grow an optional half that means nothing in the other's mode.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct BitFieldSpec {
    /// Field name, unique within its register.
    pub name: String,
    /// Bit offset of the field's least significant bit.
    pub shift: u8,
    /// Field width in bits. Default 1 — the overwhelmingly common flag.
    #[serde(default = "one_u8")]
    pub width_bits: u8,
}

impl BitFieldSpec {
    /// The field's mask in the register word.
    pub fn mask(&self) -> u32 {
        if self.shift >= 32 {
            return 0;
        }
        let width = self.width_bits.clamp(1, 32 - self.shift.min(31)) as u32;
        let base: u32 = if width >= 32 { !0 } else { (1u32 << width) - 1 };
        base << self.shift
    }
}

// ─── pin edges ─────────────────────────────────────────────────────────────

/// Which transition of an observed pad raises a `pin:` event.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PinEdge {
    Rising,
    Falling,
    /// Either edge. The default, because a bit-banged protocol usually cares
    /// about both and saying so should not be the verbose option.
    #[default]
    Any,
}

impl PinEdge {
    /// Whether a transition from `was` to `now` matches this edge.
    pub fn matches(&self, was: bool, now: bool) -> bool {
        match self {
            PinEdge::Rising => !was && now,
            PinEdge::Falling => was && !now,
            PinEdge::Any => was != now,
        }
    }
}

// ─── events ────────────────────────────────────────────────────────────────

/// What a rule fires on.
///
/// Serialised as a YAML scalar for the bare events (`start`) and as a
/// single-key map for the parameterised ones (`{ write: PWR_MGMT_1 }`), which
/// is how the plan's target document is written. `pin:` additionally accepts a
/// sibling `edge:` key, so `{ pin: SCK, edge: rising }` reads the way the
/// datasheet says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The master wrote a register (`write: REG`), or wrote a value that leaves
    /// any bit of a named field set (`write: REG.FIELD`).
    Write {
        register: String,
        field: Option<String>,
    },
    /// The master read a register.
    Read { register: String },
    /// I²C START / repeated START.
    Start,
    /// I²C STOP.
    Stop,
    /// SPI CS went low.
    CsSelect,
    /// SPI CS went high.
    CsRelease,
    /// A framing unit completed (see [`FrameSpec`]).
    Frame,
    /// A [`TimerSpec`] elapsed.
    Timer { name: String },
    /// An observed pad changed level.
    Pin { name: String, edge: PinEdge },
    /// A SimInput channel was driven.
    Input { key: String },
}

impl Event {
    /// The bare (scalar-spelled) events, in one table so the parser, the
    /// serialiser and the docs cannot disagree.
    const BARE: &'static [(&'static str, Event)] = &[
        ("start", Event::Start),
        ("stop", Event::Stop),
        ("cs_select", Event::CsSelect),
        ("cs_release", Event::CsRelease),
        ("frame", Event::Frame),
    ];

    fn bare_name(&self) -> Option<&'static str> {
        Self::BARE
            .iter()
            .find(|(_, e)| e == self)
            .map(|(name, _)| *name)
    }
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde_yaml::{Mapping, Value};
        if let Some(name) = self.bare_name() {
            return s.serialize_str(name);
        }
        let mut m = Mapping::new();
        match self {
            Event::Write { register, field } => {
                let key = match field {
                    Some(f) => format!("{register}.{f}"),
                    None => register.clone(),
                };
                m.insert(Value::from("write"), Value::from(key));
            }
            Event::Read { register } => {
                m.insert(Value::from("read"), Value::from(register.clone()));
            }
            Event::Timer { name } => {
                m.insert(Value::from("timer"), Value::from(name.clone()));
            }
            Event::Input { key } => {
                m.insert(Value::from("input"), Value::from(key.clone()));
            }
            Event::Pin { name, edge } => {
                m.insert(Value::from("pin"), Value::from(name.clone()));
                let edge = match edge {
                    PinEdge::Rising => "rising",
                    PinEdge::Falling => "falling",
                    PinEdge::Any => "any",
                };
                m.insert(Value::from("edge"), Value::from(edge));
            }
            _ => unreachable!("bare events took the scalar path"),
        }
        Value::Mapping(m).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_yaml::Value::deserialize(d)?;
        if let Some(s) = v.as_str() {
            return Event::BARE
                .iter()
                .find(|(name, _)| *name == s)
                .map(|(_, e)| e.clone())
                .ok_or_else(|| {
                    D::Error::custom(format!(
                        "`on: {s}` is not an event. Bare events are start, stop, cs_select, \
                         cs_release, frame; the rest are single-key maps: write:, read:, \
                         timer:, pin:, input:"
                    ))
                });
        }
        let map = v.as_mapping().ok_or_else(|| {
            D::Error::custom(
                "`on:` must be an event name or a single-key map such as `{ write: REG }`",
            )
        })?;
        let get = |k: &str| map.get(serde_yaml::Value::from(k));
        let as_name = |k: &str, val: &serde_yaml::Value| -> Result<String, D::Error> {
            val.as_str()
                .map(str::to_string)
                .ok_or_else(|| D::Error::custom(format!("`on: {{ {k}: … }}` needs a name")))
        };
        if let Some(val) = get("write") {
            let spelled = as_name("write", val)?;
            let (register, field) = match spelled.split_once('.') {
                Some((r, f)) => (r.to_string(), Some(f.to_string())),
                None => (spelled, None),
            };
            return Ok(Event::Write { register, field });
        }
        if let Some(val) = get("read") {
            return Ok(Event::Read {
                register: as_name("read", val)?,
            });
        }
        if let Some(val) = get("timer") {
            return Ok(Event::Timer {
                name: as_name("timer", val)?,
            });
        }
        if let Some(val) = get("input") {
            return Ok(Event::Input {
                key: as_name("input", val)?,
            });
        }
        if let Some(val) = get("pin") {
            let name = as_name("pin", val)?;
            let edge = match get("edge").and_then(|e| e.as_str()) {
                None => PinEdge::Any,
                Some("rising") => PinEdge::Rising,
                Some("falling") => PinEdge::Falling,
                Some("any") => PinEdge::Any,
                Some(other) => {
                    return Err(D::Error::custom(format!(
                        "`edge: {other}` is not rising, falling or any"
                    )))
                }
            };
            return Ok(Event::Pin { name, edge });
        }
        Err(D::Error::custom(
            "unknown event; expected one of write:, read:, timer:, pin:, input:, or the bare \
             start / stop / cs_select / cs_release / frame",
        ))
    }
}

// ─── actions ───────────────────────────────────────────────────────────────

/// Which bits an action touches: either a named field (`INT_STATUS.DATA_RDY`)
/// or an explicit `{ register, mask }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegBits {
    pub register: String,
    /// A named [`BitFieldSpec`] of that register. Resolved at load.
    pub field: Option<String>,
    /// An explicit mask, when the part has no name for the bits.
    pub mask: Option<u32>,
}

impl Serialize for RegBits {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde_yaml::{Mapping, Value};
        match (&self.field, self.mask) {
            (Some(f), None) => s.serialize_str(&format!("{}.{}", self.register, f)),
            _ => {
                let mut m = Mapping::new();
                m.insert(Value::from("register"), Value::from(self.register.clone()));
                if let Some(f) = &self.field {
                    m.insert(Value::from("field"), Value::from(f.clone()));
                }
                if let Some(mask) = self.mask {
                    m.insert(Value::from("mask"), Value::from(mask));
                }
                Value::Mapping(m).serialize(s)
            }
        }
    }
}

fn reg_bits_from_value<E: serde::de::Error>(
    key: &str,
    v: &serde_yaml::Value,
) -> Result<RegBits, E> {
    if let Some(s) = v.as_str() {
        let (register, field) = match s.split_once('.') {
            Some((r, f)) => (r.to_string(), Some(f.to_string())),
            None => (s.to_string(), None),
        };
        if field.is_none() {
            return Err(E::custom(format!(
                "`{key}: {s}` names a register but no bits. Write `{key}: {s}.FIELD` or \
                 `{key}: {{ register: {s}, mask: 0x.. }}`"
            )));
        }
        return Ok(RegBits {
            register,
            field,
            mask: None,
        });
    }
    let m = v.as_mapping().ok_or_else(|| {
        E::custom(format!(
            "`{key}:` must be REG.FIELD or {{ register, mask }}"
        ))
    })?;
    let register = m
        .get(serde_yaml::Value::from("register"))
        .and_then(|r| r.as_str())
        .ok_or_else(|| E::custom(format!("`{key}: {{ … }}` is missing `register:`")))?
        .to_string();
    let field = m
        .get(serde_yaml::Value::from("field"))
        .and_then(|f| f.as_str())
        .map(str::to_string);
    let mask = m
        .get(serde_yaml::Value::from("mask"))
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    if field.is_none() && mask.is_none() {
        return Err(E::custom(format!(
            "`{key}: {{ register: {register} }}` touches no bits — give it `mask:` or `field:`"
        )));
    }
    Ok(RegBits {
        register,
        field,
        mask,
    })
}

/// What a rule does.
///
/// Each variant is spelled as a single-key map with optional sibling keys, the
/// way the plan's target document writes them: `{ pin: INT, level: 1 }`,
/// `{ timer: sample, start: true }`, `{ var: count, value: "var(count) + 1" }`.
/// The nested spellings (`{ pin: { name: INT, level: 1 } }`) are accepted too,
/// because a generator that emits the more regular shape should not be wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Set the named bits.
    Set(RegBits),
    /// Clear the named bits.
    Clear(RegBits),
    /// Store an expression's value into a register.
    Write { register: String, value: String },
    /// Move to another declared state.
    Goto { state: String },
    /// Start (`start: true`) or stop (`start: false`) a declared timer.
    Timer { name: String, start: bool },
    /// Enqueue into a FIFO: `value:` when given, else the FIFO's `source:`.
    Push { fifo: String, value: Option<String> },
    /// Discard the oldest FIFO entry.
    Pop { fifo: String },
    /// Drive one of the part's `outputs:` pins. `level` is an EXPRESSION, not a
    /// flag: `level: 1`, `level: 0`, and `level: "field(PORT.P3)"` are all
    /// legal, and the last is what makes a bit-mapped part (an I/O expander)
    /// eight rules instead of sixteen. Non-zero drives the pin high.
    Pin { name: String, level: String },
    /// Assign a variable.
    Var { name: String, value: String },
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde_yaml::{Mapping, Value};
        let mut m = Mapping::new();
        match self {
            Action::Set(bits) => {
                m.insert(
                    Value::from("set"),
                    serde_yaml::to_value(bits).map_err(serde::ser::Error::custom)?,
                );
            }
            Action::Clear(bits) => {
                m.insert(
                    Value::from("clear"),
                    serde_yaml::to_value(bits).map_err(serde::ser::Error::custom)?,
                );
            }
            Action::Write { register, value } => {
                let mut inner = Mapping::new();
                inner.insert(Value::from("register"), Value::from(register.clone()));
                inner.insert(Value::from("value"), Value::from(value.clone()));
                m.insert(Value::from("write"), Value::Mapping(inner));
            }
            Action::Goto { state } => {
                m.insert(Value::from("goto"), Value::from(state.clone()));
            }
            Action::Timer { name, start } => {
                m.insert(Value::from("timer"), Value::from(name.clone()));
                m.insert(Value::from("start"), Value::from(*start));
            }
            Action::Push { fifo, value } => {
                m.insert(Value::from("push"), Value::from(fifo.clone()));
                if let Some(v) = value {
                    m.insert(Value::from("value"), Value::from(v.clone()));
                }
            }
            Action::Pop { fifo } => {
                m.insert(Value::from("pop"), Value::from(fifo.clone()));
            }
            Action::Pin { name, level } => {
                m.insert(Value::from("pin"), Value::from(name.clone()));
                // A constant level round-trips as the integer it was written
                // as, so a shipped descriptor does not grow quotes on a rewrite.
                m.insert(
                    Value::from("level"),
                    match level.trim() {
                        "0" => Value::from(0u64),
                        "1" => Value::from(1u64),
                        other => Value::from(other.to_string()),
                    },
                );
            }
            Action::Var { name, value } => {
                m.insert(Value::from("var"), Value::from(name.clone()));
                m.insert(Value::from("value"), Value::from(value.clone()));
            }
        }
        Value::Mapping(m).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_yaml::Value::deserialize(d)?;
        let map = v
            .as_mapping()
            .ok_or_else(|| D::Error::custom("an action must be a map such as `{ goto: idle }`"))?;
        let get = |k: &str| map.get(serde_yaml::Value::from(k));
        // `{ pin: { name: X, level: 1 } }` — the nested spelling: read the
        // sibling keys out of the inner map instead of the outer one.
        let nested = |k: &str| -> Option<&serde_yaml::Mapping> {
            get(k).and_then(|x| x.as_mapping()).filter(|inner| {
                inner.contains_key(serde_yaml::Value::from("name"))
                    || inner.contains_key(serde_yaml::Value::from("fifo"))
            })
        };
        let scalar = |k: &str, inner: Option<&serde_yaml::Mapping>| -> Option<String> {
            match inner {
                Some(m) => m
                    .get(serde_yaml::Value::from("name"))
                    .or_else(|| m.get(serde_yaml::Value::from("fifo")))
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                None => get(k).and_then(|x| x.as_str()).map(str::to_string),
            }
        };
        let sibling =
            |key: &str, inner: Option<&serde_yaml::Mapping>| -> Option<serde_yaml::Value> {
                match inner {
                    Some(m) => m.get(serde_yaml::Value::from(key)).cloned(),
                    None => get(key).cloned(),
                }
            };

        if let Some(v) = get("set") {
            return Ok(Action::Set(reg_bits_from_value::<D::Error>("set", v)?));
        }
        if let Some(v) = get("clear") {
            return Ok(Action::Clear(reg_bits_from_value::<D::Error>("clear", v)?));
        }
        if let Some(v) = get("write") {
            let m = v.as_mapping().ok_or_else(|| {
                D::Error::custom("`write:` must be `{ register: R, value: \"<expr>\" }`")
            })?;
            let register = m
                .get(serde_yaml::Value::from("register"))
                .and_then(|x| x.as_str())
                .ok_or_else(|| D::Error::custom("`write:` is missing `register:`"))?
                .to_string();
            let value = m
                .get(serde_yaml::Value::from("value"))
                .ok_or_else(|| D::Error::custom("`write:` is missing `value:`"))?;
            return Ok(Action::Write {
                register,
                value: yaml_expr::<D::Error>("write.value", value)?,
            });
        }
        if let Some(v) = get("goto") {
            return Ok(Action::Goto {
                state: v
                    .as_str()
                    .ok_or_else(|| D::Error::custom("`goto:` needs a state name"))?
                    .to_string(),
            });
        }
        if get("timer").is_some() {
            let inner = nested("timer");
            let name = scalar("timer", inner)
                .ok_or_else(|| D::Error::custom("`timer:` needs a timer name"))?;
            let start = sibling("start", inner)
                .and_then(|x| x.as_bool())
                .unwrap_or(true);
            return Ok(Action::Timer { name, start });
        }
        if get("push").is_some() {
            let inner = nested("push");
            let fifo = scalar("push", inner)
                .ok_or_else(|| D::Error::custom("`push:` needs a FIFO name"))?;
            let value = match sibling("value", inner) {
                Some(v) => Some(yaml_expr::<D::Error>("push.value", &v)?),
                None => None,
            };
            return Ok(Action::Push { fifo, value });
        }
        if let Some(v) = get("pop") {
            return Ok(Action::Pop {
                fifo: v
                    .as_str()
                    .ok_or_else(|| D::Error::custom("`pop:` needs a FIFO name"))?
                    .to_string(),
            });
        }
        if get("pin").is_some() {
            let inner = nested("pin");
            let name =
                scalar("pin", inner).ok_or_else(|| D::Error::custom("`pin:` needs a pin name"))?;
            let level = match sibling("level", inner) {
                Some(v) => match (v.as_bool(), v.as_str()) {
                    (Some(b), _) => u8::from(b).to_string(),
                    (None, Some(expr)) => expr.to_string(),
                    _ => yaml_expr::<D::Error>("pin.level", &v)?,
                },
                None => return Err(D::Error::custom("`pin:` is missing `level:`")),
            };
            return Ok(Action::Pin { name, level });
        }
        if get("var").is_some() {
            let inner = nested("var");
            let name = scalar("var", inner)
                .ok_or_else(|| D::Error::custom("`var:` needs a variable name"))?;
            let value = sibling("value", inner)
                .ok_or_else(|| D::Error::custom("`var:` is missing `value:`"))?;
            return Ok(Action::Var {
                name,
                value: yaml_expr::<D::Error>("var.value", &value)?,
            });
        }
        Err(D::Error::custom(
            "unknown action; expected one of set:, clear:, write:, goto:, timer:, push:, pop:, \
             pin:, var:",
        ))
    }
}

/// Accept an expression written either as a quoted string or as a bare YAML
/// integer (`value: 1` is far more natural than `value: "1"`).
fn yaml_expr<E: serde::de::Error>(what: &str, v: &serde_yaml::Value) -> Result<String, E> {
    if let Some(s) = v.as_str() {
        return Ok(s.to_string());
    }
    if let Some(n) = v.as_i64() {
        return Ok(n.to_string());
    }
    Err(E::custom(format!(
        "`{what}` must be an expression string or an integer"
    )))
}

// ─── the rule ──────────────────────────────────────────────────────────────

/// One event → guard → actions rule.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Rule {
    /// What fires it.
    pub on: Event,
    /// Integer guard; absent ⇒ always fires.
    #[serde(default)]
    pub when: Option<String>,
    /// Actions, run in order. `do` is a Rust keyword, hence the rename.
    #[serde(rename = "do", default)]
    pub actions: Vec<Action>,
}

// ─── compilation ───────────────────────────────────────────────────────────

/// A rule with every expression already parsed.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub on: Event,
    pub when: Option<Expr>,
    pub actions: Vec<CompiledAction>,
}

/// An action with every expression already parsed.
#[derive(Debug, Clone)]
pub enum CompiledAction {
    Set(RegBits),
    Clear(RegBits),
    Write { register: String, value: Expr },
    Goto { state: String },
    Timer { name: String, start: bool },
    Push { fifo: String, value: Option<Expr> },
    Pop { fifo: String },
    Pin { name: String, level: Expr },
    Var { name: String, value: Expr },
}

/// A rule that would not compile. Names the rule INDEX, because a rule has no
/// other identity — that index is what a part author counts down their `rules:`
/// list to find.
#[derive(Debug, Clone)]
pub struct RuleCompileError {
    /// 0-based index into `behavior.rules`.
    pub rule: usize,
    /// Which slot of the rule (`when`, `do[2].value`, …).
    pub slot: String,
    /// The expression that failed.
    pub source_text: String,
    /// Why.
    pub error: ExprError,
}

impl std::fmt::Display for RuleCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rules[{}].{}: {} — in `{}`",
            self.rule, self.slot, self.error, self.source_text
        )
    }
}

impl std::error::Error for RuleCompileError {}

/// Parse every expression in `rules`, ONCE. The engine never sees a string.
pub fn compile_rules(rules: &[Rule]) -> Result<Vec<CompiledRule>, RuleCompileError> {
    let mut out = Vec::with_capacity(rules.len());
    for (i, rule) in rules.iter().enumerate() {
        let parse = |slot: String, src: &str| -> Result<Expr, RuleCompileError> {
            Expr::parse(src).map_err(|error| RuleCompileError {
                rule: i,
                slot,
                source_text: src.to_string(),
                error,
            })
        };
        let when = match &rule.when {
            Some(src) => Some(parse("when".into(), src)?),
            None => None,
        };
        let mut actions = Vec::with_capacity(rule.actions.len());
        for (j, action) in rule.actions.iter().enumerate() {
            actions.push(match action {
                Action::Set(b) => CompiledAction::Set(b.clone()),
                Action::Clear(b) => CompiledAction::Clear(b.clone()),
                Action::Write { register, value } => CompiledAction::Write {
                    register: register.clone(),
                    value: parse(format!("do[{j}].write.value"), value)?,
                },
                Action::Goto { state } => CompiledAction::Goto {
                    state: state.clone(),
                },
                Action::Timer { name, start } => CompiledAction::Timer {
                    name: name.clone(),
                    start: *start,
                },
                Action::Push { fifo, value } => CompiledAction::Push {
                    fifo: fifo.clone(),
                    value: match value {
                        Some(v) => Some(parse(format!("do[{j}].push.value"), v)?),
                        None => None,
                    },
                },
                Action::Pop { fifo } => CompiledAction::Pop { fifo: fifo.clone() },
                Action::Pin { name, level } => CompiledAction::Pin {
                    name: name.clone(),
                    level: parse(format!("do[{j}].pin.level"), level)?,
                },
                Action::Var { name, value } => CompiledAction::Var {
                    name: name.clone(),
                    value: parse(format!("do[{j}].var.value"), value)?,
                },
            });
        }
        out.push(CompiledRule {
            on: rule.on.clone(),
            when,
            actions,
        });
    }
    Ok(out)
}

/// Static validation of the Tier-2 half of a `behavior:` block: every name a
/// rule mentions must be declared somewhere.
///
/// This is the check that turns a typo into a load error instead of a silent
/// zero. [`crate::expr`] is deliberately total — `reg(TYPO)` evaluates to 0
/// rather than failing — so without this a misspelled register would produce a
/// guard that is quietly always false. Totality at run time, strictness at load
/// time: the two together are what make a rule both safe and honest.
pub struct RuleNames<'a> {
    pub registers: &'a [String],
    pub fields: &'a [(String, String)],
    pub states: &'a [String],
    pub vars: &'a [String],
    pub fifos: &'a [String],
    pub timers: &'a [String],
    pub outputs: &'a [String],
    pub inputs: &'a [String],
    pub pins: &'a [String],
}

/// Check every rule's names against what the part declares.
pub fn validate_rule_names(rules: &[Rule], names: &RuleNames<'_>) -> anyhow::Result<()> {
    let has = |set: &[String], want: &str| set.iter().any(|s| s == want);
    let has_field =
        |reg: &str, field: &str| names.fields.iter().any(|(r, f)| r == reg && f == field);
    for (i, rule) in rules.iter().enumerate() {
        let at = |what: &str| format!("rules[{i}] ({what})");
        match &rule.on {
            Event::Write { register, field } => {
                anyhow::ensure!(
                    has(names.registers, register),
                    "{}: no register named '{register}'",
                    at("on.write")
                );
                if let Some(f) = field {
                    anyhow::ensure!(
                        has_field(register, f),
                        "{}: register '{register}' declares no `bits:` field '{f}'",
                        at("on.write")
                    );
                }
            }
            Event::Read { register } => anyhow::ensure!(
                has(names.registers, register),
                "{}: no register named '{register}'",
                at("on.read")
            ),
            Event::Timer { name } => anyhow::ensure!(
                has(names.timers, name),
                "{}: no timer named '{name}'",
                at("on.timer")
            ),
            Event::Input { key } => anyhow::ensure!(
                has(names.inputs, key),
                "{}: no input channel named '{key}'",
                at("on.input")
            ),
            Event::Pin { name, .. } => anyhow::ensure!(
                has(names.pins, name) || has(names.outputs, name),
                "{}: no pin role named '{name}' in `pins:` or `outputs:`",
                at("on.pin")
            ),
            Event::Start | Event::Stop | Event::CsSelect | Event::CsRelease | Event::Frame => {}
        }
        for (j, action) in rule.actions.iter().enumerate() {
            let at = |what: &str| format!("rules[{i}].do[{j}] ({what})");
            let check_bits = |bits: &RegBits, what: &str| -> anyhow::Result<()> {
                anyhow::ensure!(
                    has(names.registers, &bits.register),
                    "{}: no register named '{}'",
                    at(what),
                    bits.register
                );
                if let Some(f) = &bits.field {
                    anyhow::ensure!(
                        has_field(&bits.register, f),
                        "{}: register '{}' declares no `bits:` field '{f}'",
                        at(what),
                        bits.register
                    );
                }
                Ok(())
            };
            match action {
                Action::Set(b) => check_bits(b, "set")?,
                Action::Clear(b) => check_bits(b, "clear")?,
                Action::Write { register, .. } => anyhow::ensure!(
                    has(names.registers, register),
                    "{}: no register named '{register}'",
                    at("write")
                ),
                Action::Goto { state } => anyhow::ensure!(
                    has(names.states, state),
                    "{}: no state named '{state}' in `states:`",
                    at("goto")
                ),
                Action::Timer { name, .. } => anyhow::ensure!(
                    has(names.timers, name),
                    "{}: no timer named '{name}'",
                    at("timer")
                ),
                Action::Push { fifo, .. } | Action::Pop { fifo } => anyhow::ensure!(
                    has(names.fifos, fifo),
                    "{}: no FIFO named '{fifo}'",
                    at("push/pop")
                ),
                Action::Pin { name, .. } => anyhow::ensure!(
                    has(names.outputs, name),
                    "{}: no pin named '{name}' in `outputs:` — a rule may only drive a \
                     declared output",
                    at("pin")
                ),
                Action::Var { name, .. } => anyhow::ensure!(
                    has(names.vars, name),
                    "{}: no variable named '{name}' in `vars:`",
                    at("var")
                ),
            }
        }
    }
    Ok(())
}

/// Every name a set of rules reads through `reg()` / `field()`, so a caller can
/// check them against the declared register map.
pub fn rule_expression_registers(rules: &[CompiledRule]) -> Vec<String> {
    let mut out = Vec::new();
    for rule in rules {
        if let Some(w) = &rule.when {
            w.registers(&mut out);
        }
        for a in &rule.actions {
            match a {
                CompiledAction::Write { value, .. } | CompiledAction::Var { value, .. } => {
                    value.registers(&mut out)
                }
                CompiledAction::Push {
                    value: Some(value), ..
                } => value.registers(&mut out),
                CompiledAction::Pin { level, .. } => level.registers(&mut out),
                _ => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Reset values for a part's variables, in declaration order.
pub type VarResets = BTreeMap<String, i64>;

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_from(yaml: &str) -> Vec<Rule> {
        serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("parse rules: {e}"))
    }

    #[test]
    fn the_plans_target_document_parses() {
        let rules = rules_from(
            r#"
- on: { write: PWR_MGMT_1 }
  when: "reg(PWR_MGMT_1) & 0x40 == 0"
  do: [ { timer: sample, start: true } ]
- on: { timer: sample }
  do: [ { set: INT_STATUS.DATA_RDY }, { pin: INT, level: 1 } ]
- on: { read: INT_STATUS }
  do: [ { pin: INT, level: 0 } ]
"#,
        );
        assert_eq!(rules.len(), 3);
        assert_eq!(
            rules[0].on,
            Event::Write {
                register: "PWR_MGMT_1".into(),
                field: None
            }
        );
        assert_eq!(
            rules[0].actions[0],
            Action::Timer {
                name: "sample".into(),
                start: true
            }
        );
        assert_eq!(
            rules[1].actions[1],
            Action::Pin {
                name: "INT".into(),
                level: "1".into()
            }
        );
        compile_rules(&rules).expect("the plan's rules compile");
    }

    #[test]
    fn every_event_shape_round_trips() {
        let rules = rules_from(
            r#"
- { on: start, do: [] }
- { on: stop, do: [] }
- { on: cs_select, do: [] }
- { on: cs_release, do: [] }
- { on: frame, do: [] }
- { on: { write: A.B }, do: [] }
- { on: { read: A }, do: [] }
- { on: { timer: t }, do: [] }
- { on: { input: k }, do: [] }
- { on: { pin: P, edge: rising }, do: [] }
- { on: { pin: P, edge: falling }, do: [] }
- { on: { pin: P }, do: [] }
"#,
        );
        assert_eq!(rules.len(), 12);
        assert_eq!(
            rules[5].on,
            Event::Write {
                register: "A".into(),
                field: Some("B".into())
            }
        );
        assert_eq!(
            rules[9].on,
            Event::Pin {
                name: "P".into(),
                edge: PinEdge::Rising
            }
        );
        assert_eq!(
            rules[11].on,
            Event::Pin {
                name: "P".into(),
                edge: PinEdge::Any
            }
        );
        // The canonicalisation part_pack.rs performs must survive a round trip:
        // a pack is interned by its own re-serialised bytes.
        let yaml = serde_yaml::to_string(&rules).unwrap();
        let again: Vec<Rule> = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(rules, again, "rules must round-trip through serde_yaml");
    }

    #[test]
    fn every_action_shape_round_trips() {
        let rules = rules_from(
            r#"
- on: frame
  do:
    - { set: A.FLAG }
    - { clear: { register: A, mask: 0x30 } }
    - { write: { register: A, value: "written + 1" } }
    - { goto: armed }
    - { timer: t, start: false }
    - { push: f }
    - { push: f, value: "input(weight)" }
    - { pop: f }
    - { pin: INT, level: 1 }
    - { pin: { name: INT, level: 0 } }
    - { pin: INT, level: "field(A.FLAG)" }
    - { var: n, value: "var(n) + 1" }
"#,
        );
        let acts = &rules[0].actions;
        assert_eq!(acts.len(), 12);
        assert_eq!(
            acts[1],
            Action::Clear(RegBits {
                register: "A".into(),
                field: None,
                mask: Some(0x30)
            })
        );
        assert_eq!(
            acts[9],
            Action::Pin {
                name: "INT".into(),
                level: "0".into()
            },
            "the nested spelling means the same thing"
        );
        let yaml = serde_yaml::to_string(&rules).unwrap();
        let again: Vec<Rule> = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(rules, again);
        compile_rules(&rules).unwrap();
    }

    #[test]
    fn a_bad_expression_names_the_rule_and_the_token() {
        let rules = rules_from(
            r#"
- { on: start, do: [] }
- { on: stop, when: "reg(A) &&& 1", do: [] }
"#,
        );
        let err = compile_rules(&rules).unwrap_err();
        assert_eq!(err.rule, 1);
        assert_eq!(err.slot, "when");
        let text = err.to_string();
        assert!(text.contains("rules[1].when"), "{text}");
        assert!(text.contains("reg(A) &&& 1"), "{text}");
    }

    #[test]
    fn a_bad_action_expression_names_its_slot() {
        let rules = rules_from(
            r#"
- on: frame
  do:
    - { goto: idle }
    - { var: n, value: "1 + " }
"#,
        );
        let err = compile_rules(&rules).unwrap_err();
        assert_eq!(err.slot, "do[1].var.value");
    }

    #[test]
    fn undeclared_names_are_a_load_error() {
        let rules = rules_from(r#"[{ on: { timer: nope }, do: [] }]"#);
        let empty: [String; 0] = [];
        let fields: [(String, String); 0] = [];
        let names = RuleNames {
            registers: &empty,
            fields: &fields,
            states: &empty,
            vars: &empty,
            fifos: &empty,
            timers: &empty,
            outputs: &empty,
            inputs: &empty,
            pins: &empty,
        };
        let err = validate_rule_names(&rules, &names).unwrap_err();
        assert!(err.to_string().contains("no timer named 'nope'"), "{err}");
    }

    #[test]
    fn a_rule_may_only_drive_a_declared_output() {
        let rules = rules_from(r#"[{ on: frame, do: [ { pin: INT, level: 1 } ] }]"#);
        let empty: [String; 0] = [];
        let fields: [(String, String); 0] = [];
        let names = RuleNames {
            registers: &empty,
            fields: &fields,
            states: &empty,
            vars: &empty,
            fifos: &empty,
            timers: &empty,
            outputs: &empty,
            inputs: &empty,
            pins: &empty,
        };
        let err = validate_rule_names(&rules, &names).unwrap_err();
        assert!(err.to_string().contains("`outputs:`"), "{err}");
    }

    #[test]
    fn bit_field_masks() {
        let f = BitFieldSpec {
            name: "X".into(),
            shift: 3,
            width_bits: 2,
        };
        assert_eq!(f.mask(), 0b11000);
        let one = BitFieldSpec {
            name: "Y".into(),
            shift: 0,
            width_bits: 1,
        };
        assert_eq!(one.mask(), 1);
    }

    #[test]
    fn pin_edges() {
        assert!(PinEdge::Rising.matches(false, true));
        assert!(!PinEdge::Rising.matches(true, false));
        assert!(PinEdge::Falling.matches(true, false));
        assert!(PinEdge::Any.matches(true, false));
        assert!(!PinEdge::Any.matches(true, true));
    }
}
