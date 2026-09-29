// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Declarative `analog_mux` primitive: an analog multiplexer between analog
//! sources and one ADC channel, described in `configs/devices/*.yaml` by its
//! select pins, its channel count and its enable. The 74HC4051
//! (`74hc4051.yaml`) is the proof part.
//!
//! ```text
//!  source ──Y0┐
//!  source ──Y1┤  mux  ──Z── ADC channel (connection / channel)
//!    ...      │   ▲ ▲ ▲ ▲
//!  source ──Yn┘  S0 S1 S2 E  ◀── MCU GPIO outputs
//! ```
//!
//! * An analog source (a potentiometer, a thermistor, ...) that names the
//!   placed mux as its `connection:` and a mux input as its `channel` drives
//!   that input.
//! * The select and enable pads are read from the GPIO model (the pad level,
//!   [`crate::Peripheral::read_gpio_pad`]). The bus re-routes inside every
//!   MMIO write to a GPIO peripheral that hosts one of these pads
//!   (`bus/analog_mux.rs`), so a conversion that the firmware starts right
//!   after it moved the select lines converts the NEW channel.
//! * Z drives the downstream ADC channel with the routed input's voltage;
//!   with the enable deasserted Z is open and the channel reads 0 mV.
//!
//! This file is the engine only: the part's facts live in its descriptor.

use anyhow::{bail, Result};
use labwired_config::{ActiveLevel, AnalogMuxSpec, DeviceDescriptor};

/// One MCU pad the mux observes, resolved at attach time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuxPad {
    /// The pad label as written in system.yaml (for diagnostics).
    pub label: String,
    /// Bus index of the GPIO peripheral that owns the pad.
    pub peripheral: usize,
    /// Pin number inside that GPIO peripheral.
    pub bit: u8,
}

/// The select decode of a binary analog mux: `select[i]` is the level of the
/// select pin of weight 2^i. `enabled` false opens every switch. Returns the
/// routed input, or `None` with all switches open.
pub fn decode(select: &[bool], enabled: bool) -> Option<usize> {
    if !enabled {
        return None;
    }
    Some(
        select
            .iter()
            .enumerate()
            .map(|(i, &s)| (s as usize) << i)
            .sum(),
    )
}

/// Check the static `analog_mux` block of a descriptor.
pub fn validate_spec(spec: &AnalogMuxSpec) -> Result<()> {
    if spec.select.is_empty() || spec.select.len() > 8 {
        bail!(
            "analog_mux: `select` must name 1..=8 pins, got {}",
            spec.select.len()
        );
    }
    let expected = 1usize << spec.select.len();
    if spec.channels != expected {
        bail!(
            "analog_mux: {} select pins address {expected} channels, but `channels` is {}",
            spec.select.len(),
            spec.channels
        );
    }
    let mut roles: Vec<&str> = spec.select.iter().map(String::as_str).collect();
    if let Some(e) = &spec.enable {
        roles.push(e.pin.as_str());
    }
    for (i, r) in roles.iter().enumerate() {
        if r.is_empty() {
            bail!("analog_mux: a pin role name is empty");
        }
        if roles[..i].contains(r) {
            bail!("analog_mux: pin role '{r}' is named twice");
        }
    }
    Ok(())
}

/// Validate a whole descriptor for the `analog_mux` primitive.
pub(crate) fn validate_descriptor(desc: &DeviceDescriptor) -> Result<()> {
    let Some(spec) = desc.behavior.analog_mux.as_ref() else {
        bail!("analog_mux '{}' has no `analog_mux:` block", desc.r#type);
    };
    validate_spec(spec)
}

/// True for a system.yaml `type:` whose embedded descriptor is an
/// `analog_mux`. Parts that name such a device as their `connection:` are its
/// analog inputs, not devices behind an I²C bus switch.
pub fn is_analog_mux_type(type_str: &str) -> bool {
    labwired_config::embedded_device_yaml(type_str)
        .and_then(|yaml| DeviceDescriptor::from_yaml(yaml).ok())
        .is_some_and(|d| d.behavior.primitive == "analog_mux")
}

/// A placed analog mux: its wiring and the level on each input.
#[derive(Debug, Clone)]
pub struct AnalogMux {
    /// system.yaml `external_devices` id. Analog sources name it as their
    /// `connection:`.
    pub id: String,
    /// Where Z goes: an ADC peripheral id, or the id of another mux.
    pub connection: String,
    /// The channel of `connection` that Z drives.
    pub channel: u8,
    /// Select pads, least significant first.
    pub select: Vec<MuxPad>,
    /// Enable pad and the level that enables. `None` = tied active.
    pub enable: Option<(MuxPad, ActiveLevel)>,
    /// Level on each input in millivolts. An input nothing drives is 0 mV.
    inputs: Vec<u16>,
    /// The last routing (`None` inside = all switches open). The outer
    /// `None` means the mux has not been routed yet.
    routed: Option<Option<usize>>,
    /// Number of times the routing changed (evidence for tests).
    switches: u64,
}

impl AnalogMux {
    /// A mux with `select.len()` select pads and `2 ^ select.len()` inputs.
    pub fn new(
        id: impl Into<String>,
        connection: impl Into<String>,
        channel: u8,
        select: Vec<MuxPad>,
        enable: Option<(MuxPad, ActiveLevel)>,
    ) -> Self {
        let channels = 1usize << select.len();
        Self {
            id: id.into(),
            connection: connection.into(),
            channel,
            select,
            enable,
            inputs: vec![0; channels],
            routed: None,
            switches: 0,
        }
    }

    /// Number of analog inputs.
    pub fn channels(&self) -> usize {
        self.inputs.len()
    }

    /// Set the level on input `channel`. False for an input the part does not
    /// have.
    pub fn set_input_mv(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs.get_mut(channel as usize) {
            Some(slot) => {
                *slot = millivolts;
                true
            }
            None => false,
        }
    }

    /// Level on input `channel` in millivolts.
    pub fn input_mv(&self, channel: usize) -> Option<u16> {
        self.inputs.get(channel).copied()
    }

    /// The input currently routed to Z (`None`: switches open, or the mux has
    /// not been routed yet).
    pub fn selected(&self) -> Option<usize> {
        self.routed.flatten()
    }

    /// How many times the routing changed since attach.
    pub fn switch_count(&self) -> u64 {
        self.switches
    }

    /// Record a routing. Returns the level Z now drives, and whether the
    /// routing differs from the previous one (true for the first routing).
    pub fn route(&mut self, routed: Option<usize>) -> (u16, bool) {
        let changed = self.routed != Some(routed);
        if changed && self.routed.is_some() {
            self.switches += 1;
        }
        self.routed = Some(routed);
        (self.z_mv(), changed)
    }

    /// Level on Z: the routed input, or 0 mV with all switches open.
    pub fn z_mv(&self) -> u16 {
        self.selected()
            .and_then(|ch| self.inputs.get(ch).copied())
            .unwrap_or(0)
    }

    /// True when `peripheral` hosts a select or enable pad.
    pub fn watches(&self, peripheral: usize) -> bool {
        self.select.iter().any(|p| p.peripheral == peripheral)
            || self
                .enable
                .as_ref()
                .is_some_and(|(p, _)| p.peripheral == peripheral)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(bit: u8) -> MuxPad {
        MuxPad {
            label: format!("P{bit}"),
            peripheral: 0,
            bit,
        }
    }

    fn mux(enable: bool) -> AnalogMux {
        AnalogMux::new(
            "m",
            "adc1",
            3,
            vec![pad(0), pad(1), pad(2)],
            enable.then(|| (pad(3), ActiveLevel::Low)),
        )
    }

    fn code(n: usize) -> [bool; 3] {
        [n & 1 != 0, n & 2 != 0, n & 4 != 0]
    }

    /// 74HC4051 function table: every S2 S1 S0 code selects
    /// Yn = S2*4 + S1*2 + S0 while enabled.
    #[test]
    fn every_select_code_routes_its_channel() {
        for n in 0..8 {
            assert_eq!(decode(&code(n), true), Some(n), "code {n}");
        }
    }

    /// Enable deasserted opens every switch, whatever the select code.
    #[test]
    fn disabled_opens_every_switch() {
        for n in 0..8 {
            assert_eq!(decode(&code(n), false), None, "code {n}");
        }
    }

    /// Z carries exactly the routed input: distinct levels on all eight
    /// inputs, each code reads its own level and no other.
    #[test]
    fn z_follows_the_routed_input() {
        let mut m = mux(true);
        assert_eq!(m.channels(), 8);
        for ch in 0..8u8 {
            assert!(m.set_input_mv(ch, 100 + 300 * ch as u16));
        }
        for n in 0..8 {
            assert_eq!(m.route(Some(n)).0, 100 + 300 * n as u16);
            assert_eq!(m.selected(), Some(n));
        }
        assert_eq!(m.route(None), (0, true), "switches open: Z reads 0 mV");
        assert_eq!(m.selected(), None);
        assert!(!m.set_input_mv(8, 1), "an 8-channel mux has no input 8");
    }

    #[test]
    fn switch_count_counts_changes_only() {
        let mut m = mux(false);
        assert!(m.route(Some(0)).1, "the first routing is a change");
        assert!(!m.route(Some(0)).1);
        assert_eq!(m.switch_count(), 0);
        m.route(Some(5));
        m.route(None);
        assert_eq!(m.switch_count(), 2);
    }

    #[test]
    fn watches_only_its_own_gpio_peripherals() {
        let mut m = mux(true);
        m.enable.as_mut().unwrap().0.peripheral = 4;
        assert!(m.watches(0));
        assert!(m.watches(4));
        assert!(!m.watches(1));
    }

    #[test]
    fn spec_channel_count_must_match_select_pins() {
        let spec = |select: &[&str], channels: usize| AnalogMuxSpec {
            select: select.iter().map(|s| s.to_string()).collect(),
            channels,
            enable: None,
        };
        assert!(validate_spec(&spec(&["S0", "S1", "S2"], 8)).is_ok());
        assert!(validate_spec(&spec(&["S0", "S1", "S2"], 16)).is_err());
        assert!(validate_spec(&spec(&[], 1)).is_err());
        assert!(validate_spec(&spec(&["S0", "S0"], 4)).is_err());
    }

    /// The shipped 74HC4051 descriptor is an 8-channel analog_mux with an
    /// active-low enable, under all three spellings.
    #[test]
    fn hc4051_descriptor_is_an_eight_channel_mux() {
        for t in ["74hc4051", "cd74hc4051", "cd4051"] {
            assert!(is_analog_mux_type(t), "{t}");
        }
        assert!(!is_analog_mux_type("potentiometer"));
        let d = DeviceDescriptor::from_yaml(
            labwired_config::embedded_device_yaml("74hc4051").expect("embedded"),
        )
        .unwrap();
        validate_descriptor(&d).unwrap();
        let spec = d.behavior.analog_mux.unwrap();
        assert_eq!(spec.select, ["S0", "S1", "S2"]);
        assert_eq!(spec.channels, 8);
        assert_eq!(spec.enable.unwrap().active, ActiveLevel::Low);
    }
}
