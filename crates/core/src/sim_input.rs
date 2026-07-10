//! Generic simulated-input interface.
//!
//! An "input component" is a modelled device whose behaviour an external
//! driver can steer at runtime — an accelerometer's pose, a distance sensor's
//! range, a thermistor's temperature. Historically each had a bespoke setter
//! (`Fxos8700::set_sample`, `set_hcsr04_distance`, …) reached by downcasting to
//! the concrete type, so every caller (the WASM bridge, a future MCP tool, a
//! test script) hard-coded the type list.
//!
//! `SimInput` is the one generic contract over those setters. A device declares
//! the named channels it accepts (`input_channels`) and applies a value to one
//! (`set_input`). [`crate::Machine::set_input`] resolves a channel to the
//! (unique) attached device that exposes it and drives it — so an agent can say
//! "set channel `x` to 2.0" without knowing the device type, address, or bus.
//!
//! Units are the human-facing engineering units in [`InputChannel::unit`] (g,
//! cm, °C …); each device impl owns the conversion to its raw register form, so
//! that math lives in ONE place per device.

/// Metadata for one drivable channel of an input component. Serialized
/// verbatim into the peripherals manifest (device schema), so external
/// consumers see each device's drivable channels without running a machine.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct InputChannel {
    /// Stable key used to address the channel (e.g. `"x"`, `"distance"`).
    pub key: &'static str,
    /// Human-facing label (e.g. `"X"`, `"Distance"`).
    pub label: &'static str,
    /// Engineering unit of `value` in [`SimInput::set_input`] (e.g. `"g"`).
    pub unit: &'static str,
    /// Inclusive minimum accepted value, in `unit`.
    pub min: f64,
    /// Inclusive maximum accepted value, in `unit`.
    pub max: f64,
}

/// Why a [`SimInput::set_input`] / [`crate::Machine::set_input`] call failed.
#[derive(Debug, Clone, PartialEq)]
pub enum SimInputError {
    /// No channel with this key on the resolved device.
    UnknownChannel(String),
    /// `value` is outside the channel's `[min, max]`.
    OutOfRange {
        key: String,
        value: f64,
        min: f64,
        max: f64,
    },
    /// No attached input device exposes this channel.
    NoDevice(String),
    /// More than one attached device exposes this channel; disambiguation is
    /// required (a future target selector). Carries how many matched.
    Ambiguous { channel: String, matches: usize },
}

impl core::fmt::Display for SimInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SimInputError::UnknownChannel(k) => write!(f, "unknown input channel '{k}'"),
            SimInputError::OutOfRange {
                key,
                value,
                min,
                max,
            } => write!(
                f,
                "value {value} for channel '{key}' is out of range [{min}, {max}]"
            ),
            SimInputError::NoDevice(c) => {
                write!(f, "no attached input device exposes channel '{c}'")
            }
            SimInputError::Ambiguous { channel, matches } => write!(
                f,
                "channel '{channel}' is exposed by {matches} devices; disambiguation required"
            ),
        }
    }
}

impl std::error::Error for SimInputError {}

/// A device that accepts runtime input on one or more named channels.
///
/// Implementors also override `as_sim_input_mut` on their bus-device trait
/// (e.g. `I2cDevice`) so [`crate::Machine::set_input`] can reach them without
/// downcasting to the concrete type.
pub trait SimInput {
    /// The channels this device accepts, with metadata for discovery.
    fn input_channels(&self) -> &'static [InputChannel];

    /// Apply `value` (in the channel's `unit`) to channel `key`.
    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError>;

    /// This device instance's identity: the `external_devices` id the author
    /// wrote in system.yaml (e.g. `fxos8700`), stamped at attach time via
    /// [`SimInput::set_component_id`]. Discovery reports it as the owner and
    /// the stimulus resolver matches `component` against it, so the name an
    /// author writes is the name the API speaks — and two devices on the same
    /// bus stay individually addressable. `None` for hand-built devices.
    fn component_id(&self) -> Option<&str> {
        None
    }

    /// Stamp the config-file identity onto this instance (called once at
    /// attach). Default no-op for devices that don't store one.
    fn set_component_id(&mut self, _id: String) {}

    /// Range-check helper: returns the matching channel or a typed error.
    fn require_channel(&self, key: &str, value: f64) -> Result<InputChannel, SimInputError> {
        let ch = self
            .input_channels()
            .iter()
            .find(|c| c.key == key)
            .copied()
            .ok_or_else(|| SimInputError::UnknownChannel(key.to_string()))?;
        if value < ch.min || value > ch.max {
            return Err(SimInputError::OutOfRange {
                key: key.to_string(),
                value,
                min: ch.min,
                max: ch.max,
            });
        }
        Ok(ch)
    }
}
