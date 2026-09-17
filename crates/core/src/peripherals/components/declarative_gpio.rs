// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The **`gpio_device` primitive** — a part whose whole interface is pins.
//!
//! What it covers
//! ==============
//! The family a register map cannot reach at all: a bit-banged protocol the MCU
//! clocks by hand (HX711, TM1637), a part that answers a pin with a pin, a
//! button with logic behind it. Before this, each was a hand-written Rust model
//! with its own service pass and its own `Vec` on the bus. With it, such a part
//! is a YAML file: `pins:` are the pads the MCU drives and this device observes,
//! `outputs:` are the pads this device drives, `timers:` give it its own clock,
//! and [`rules`](labwired_config::Rule) tie them together.
//!
//! It is one [`BusResidentDevice`] that owns one
//! [`RuleMachine`](super::rule_machine::RuleMachine), so it joins the SAME
//! per-tick service pass the DHT22, the encoder and the keypad already share
//! and reaches its pads through the same narrowed [`DevicePins`] port. No new
//! engine type crosses that boundary — `tests/bus_resident_device_port.rs` is
//! what holds that line, and this primitive adds nothing to the trait.
//!
//! Its clock
//! =========
//! Pin-only parts are not on a data bus, so nothing hands them microseconds.
//! They get device time the way every other self-timed resident device does:
//! from the simulated cycle count and the system's `cpu_hz`, converted here and
//! carried with a remainder so a slow tick rate does not quietly lose time. That
//! is the same derived clock Phase A gave the bus devices, and it carries the
//! same fidelity note — a PLL reconfiguration is not tracked.
//!
//! What it deliberately is NOT
//! ==========================
//! A replacement for the irreducible primitives. `quadrature`, `matrix`,
//! `one_wire` and `pulse_echo` each carry a genuine algorithm (a Gray walk, a
//! reflect, a frame, an echo deadline) that a rule list would have to spell out
//! bit by bit. This primitive is for parts whose behaviour IS a small state
//! machine over edges — which, it turns out, is most of the bit-banged catalog.

use std::collections::BTreeMap;

use anyhow::{anyhow, Context, Result};
use labwired_config::{DeviceDescriptor, Event, PinEdge};

use super::declarative_regs::{apply_timing_action, TimerBank};
use super::rule_machine::{PinOnlyCtx, RuleMachine};
use crate::bus::{BusResidentDevice, DevicePins};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// One pad this device watches or drives, resolved to `(address, bit)` at
/// attach. The role name is what a rule says; the address is what the bus needs.
#[derive(Debug, Clone)]
pub struct BoundPin {
    /// Descriptor role name (`SCK`, `DOUT`).
    pub role: String,
    /// Resolved register address — ODR for an observed pin, IDR for a driven one.
    pub addr: u64,
    pub bit: u8,
}

/// A pins-only declarative device.
#[derive(Debug)]
pub struct DeclarativeGpioDevice {
    id: String,
    machine: RuleMachine,
    /// The part's own timers — the SAME [`TimerBank`] the bus devices use, so a
    /// pins-only part's clock is not a second implementation that can drift.
    /// A pins-only part has no register file, so a timer's `on_fire:` actions
    /// have nowhere to land and are dropped; what a rule listens for is the
    /// `timer:` EVENT, which is the whole point of a timer here.
    timers: TimerBank,
    /// Pads the MCU drives and this device observes (ODR).
    observed: Vec<BoundPin>,
    /// Last level seen on each observed pad; `None` until the first service.
    last_seen: Vec<Option<bool>>,
    /// Pads this device drives (IDR + the external-world seam).
    driven: Vec<BoundPin>,
    /// Measurement slots in engineering units, keyed by input-channel key.
    slots: BTreeMap<String, f64>,
    channels: &'static [InputChannel],
    cpu_hz: u64,
    /// Simulated cycle at the previous service, for the derived clock.
    last_cycle: Option<u64>,
    /// Cycles not yet converted into a whole microsecond. Without this a tick
    /// interval shorter than one µs would convert to 0 µs EVERY time and the
    /// device's clock would never move at all.
    cycle_remainder: u64,
    /// Device time in µs, derived from cycles above.
    elapsed_us: u64,
}

impl DeclarativeGpioDevice {
    /// Build from a descriptor whose pins have already been resolved to pads.
    pub fn new(
        id: String,
        descriptor: &DeviceDescriptor,
        observed: Vec<BoundPin>,
        driven: Vec<BoundPin>,
        cpu_hz: u64,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        let machine = RuleMachine::from_behavior(&descriptor.behavior)?.ok_or_else(|| {
            anyhow!(
                "gpio_device '{}' declares no rules, timers or outputs — a pins-only part with \
                 no behaviour would attach, answer nothing, and look like a working device",
                descriptor.r#type
            )
        })?;
        let mut slots = BTreeMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
            }
        }
        Ok(Self {
            id,
            machine,
            timers: TimerBank::new(&descriptor.behavior.timers),
            last_seen: vec![None; observed.len()],
            observed,
            driven,
            slots,
            channels,
            cpu_hz: cpu_hz.max(1),
            last_cycle: None,
            cycle_remainder: 0,
            elapsed_us: 0,
        })
    }

    /// Seed a measurement slot from a `config:` override, like every other
    /// declarative primitive does.
    pub fn seed_input(&mut self, key: &str, value: f64) {
        if self.channels.iter().any(|c| c.key == key) {
            self.slots.insert(key.to_string(), value);
        }
    }

    /// Read-only view of the rule machine, for tests and diagnostics.
    pub fn rule_machine(&self) -> &RuleMachine {
        &self.machine
    }

    fn fire(&mut self, event: Event) {
        let mut ctx = PinOnlyCtx { slots: &self.slots };
        self.machine.fire(&event, 0, &mut ctx);
    }

    /// Convert elapsed CYCLES into whole microseconds, keeping the remainder.
    fn advance_clock(&mut self, now: u64) {
        let Some(previous) = self.last_cycle.replace(now) else {
            return; // the first service anchors the clock; it measures nothing
        };
        // A backward jump is a reset/reanchor, not negative time.
        let delta = now.saturating_sub(previous);
        if delta == 0 {
            return;
        }
        let total = self.cycle_remainder.saturating_add(delta);
        // Below 1 MHz one cycle is worth more than a microsecond, so the
        // cycles-per-µs divisor rounds to zero; `checked_div` is what picks the
        // long way there rather than a guard someone can forget to keep.
        let (us, remainder) = match total.checked_div(self.cpu_hz / 1_000_000) {
            Some(us) => (us, total % (self.cpu_hz / 1_000_000)),
            None => {
                let us = total.saturating_mul(1_000_000) / self.cpu_hz;
                (
                    us,
                    total.saturating_sub(us.saturating_mul(self.cpu_hz) / 1_000_000),
                )
            }
        };
        self.cycle_remainder = remainder;
        if us == 0 {
            return;
        }
        self.elapsed_us = self.elapsed_us.saturating_add(us);
        self.machine.advance_time_us(us);
        if !self.timers.is_empty() {
            let mut registers = std::collections::HashMap::new();
            for (name, actions) in self.timers.due_by_timer(self.elapsed_us) {
                // A pins-only part has no register file; the actions are
                // applied to a scratch map so the ONE bank keeps one code path,
                // and the result is discarded. The `timer:` event below is what
                // a `gpio_device` rule actually listens for.
                for action in &actions {
                    apply_timing_action(action, &mut registers);
                }
                self.fire(Event::Timer { name });
            }
        }
        self.drain_timer_requests();
    }

    /// Apply whatever `timer:` actions the rules queued to the bank.
    fn drain_timer_requests(&mut self) {
        let requests = self.machine.take_timer_requests();
        for (name, start) in requests {
            if start {
                self.timers.start_named(&name, self.elapsed_us);
            } else {
                self.timers.stop_named(&name);
            }
        }
    }
}

impl BusResidentDevice for DeclarativeGpioDevice {
    /// One pass: sample the observed pads and raise an edge event for each
    /// change, advance the device's own clock (firing due timers), then put
    /// whatever the rules queued onto the driven pads.
    ///
    /// Sampling comes FIRST and driving LAST so a rule that answers an edge
    /// with a level change has that level on the pad in the same tick the edge
    /// arrived — which is what a bit-banged read loop expects: clock high, read
    /// the data line.
    fn service(&mut self, pins: &mut dyn DevicePins, now: u64) {
        for i in 0..self.observed.len() {
            let pin = self.observed[i].clone();
            // An address that does not read back means the MCU is driving
            // nothing there; `false` is the same default the other resident
            // models take for an undriven output.
            let level = pins.output_bit(pin.addr, pin.bit).unwrap_or(false);
            let was = self.last_seen[i];
            self.last_seen[i] = Some(level);
            let Some(was) = was else { continue }; // the first sample is an anchor
            if was == level {
                continue;
            }
            self.fire(Event::Pin {
                name: pin.role.clone(),
                edge: if level {
                    PinEdge::Rising
                } else {
                    PinEdge::Falling
                },
            });
            // An edge rule may have started or stopped a timer.
            self.drain_timer_requests();
        }

        self.advance_clock(now);

        for (role, level) in self.machine.take_pin_drives() {
            let Some(pin) = self.driven.iter().find(|p| p.role == role) else {
                continue;
            };
            // ⚠️ BOTH SEAMS. `drive_idr_bit` is an ordinary store to the input
            // register, which lands only where the model lets one land (STM32).
            // On silicon whose input word is READ-ONLY (EFR32, SAM, ESP32-C3)
            // the store is correctly dropped and the pad would never move —
            // `drive_input_bit` is the external-world seam those models sample.
            // Driving only one of the two is how a knob goes inert on half the
            // catalog; see the same pair in `rotary_encoder.rs`.
            let _ = pins.drive_input_bit(pin.addr, pin.bit, level);
            pins.drive_idr_bit(pin.addr, pin.bit, level);
        }
    }

    fn as_sim_input(&mut self) -> &mut dyn SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl SimInput for DeclarativeGpioDevice {
    fn input_channels(&self) -> &'static [InputChannel] {
        self.channels
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.slots.insert(key.to_string(), value);
        self.fire(Event::Input {
            key: key.to_string(),
        });
        self.drain_timer_requests();
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

/// Validate the static descriptor contract for the `gpio_device` primitive.
///
/// Kept separate from construction (like every sibling primitive) so manifest
/// preflight can reject an incomplete pack without resolving any pad.
pub(crate) fn validate_descriptor(desc: &DeviceDescriptor) -> Result<()> {
    if desc.behavior.primitive != "gpio_device" {
        anyhow::bail!(
            "declarative gpio kit requires behavior.primitive: gpio_device, got '{}'",
            desc.behavior.primitive
        );
    }
    let b = &desc.behavior;
    if b.pins.is_empty() && b.outputs.is_empty() {
        anyhow::bail!(
            "gpio_device '{}' binds no pins: a pins-only part with neither `pins:` nor \
             `outputs:` is wired to nothing",
            desc.r#type
        );
    }
    if b.rules.is_empty() {
        anyhow::bail!(
            "gpio_device '{}' declares no `rules:` — its pins would never move",
            desc.r#type
        );
    }
    for (role, key) in &b.pins {
        anyhow::ensure!(
            !key.trim().is_empty(),
            "gpio_device '{}' has a blank config key for pin role '{role}'",
            desc.r#type
        );
    }
    // Compiling the expressions here is the whole point of preflight: a
    // malformed guard must be a load error naming the rule, not a surprise at
    // the first edge.
    labwired_config::compile_rules(&b.rules)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("gpio_device '{}' has an invalid rule", desc.r#type))?;
    validate_rule_names(desc)?;
    Ok(())
}

/// Check every name a rule mentions against what the descriptor declares —
/// shared by the gpio, I²C and SPI primitives so a typo fails identically
/// whichever transport the part is on.
pub(crate) fn validate_rule_names(desc: &DeviceDescriptor) -> Result<()> {
    let b = &desc.behavior;
    let mut registers: Vec<String> = Vec::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    for spec in b
        .i2c
        .iter()
        .flat_map(|s| s.registers.iter())
        .chain(b.spi.iter().flat_map(|s| s.registers.iter()))
    {
        registers.push(spec.name.clone());
        for bit in &spec.bits {
            fields.push((spec.name.clone(), bit.name.clone()));
        }
    }
    let vars: Vec<String> = b.vars.keys().cloned().collect();
    let fifos: Vec<String> = b.fifos.iter().map(|f| f.name.clone()).collect();
    let timers: Vec<String> = b.timers.iter().map(|t| t.name.clone()).collect();
    let pins: Vec<String> = b.pins.keys().cloned().collect();
    let inputs: Vec<String> = desc
        .metadata
        .as_ref()
        .map(|m| m.inputs.iter().map(|i| i.key.clone()).collect())
        .unwrap_or_default();
    labwired_config::validate_rule_names(
        &b.rules,
        &labwired_config::RuleNames {
            registers: &registers,
            fields: &fields,
            states: &b.states,
            vars: &vars,
            fifos: &fifos,
            timers: &timers,
            outputs: &b.outputs,
            inputs: &inputs,
            pins: &pins,
        },
    )
    .with_context(|| format!("part '{}' names something it does not declare", desc.r#type))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
type: test_gpio_device
behavior:
  primitive: gpio_device
  pins: { CLK: clk_pin }
  outputs: [OUT]
  vars: { n: 0 }
  timers:
    - { name: tick, period_us: 100, start: on_reset }
  rules:
    - on: { pin: CLK, edge: rising }
      do: [ { var: n, value: "var(n) + 1" }, { pin: OUT, level: 1 } ]
    - on: { pin: CLK, edge: falling }
      do: [ { pin: OUT, level: 0 } ]
    - on: { timer: tick }
      do: [ { var: n, value: "var(n) + 100" } ]
metadata:
  inputs:
    - { key: weight, label: W, unit: g, min: 0, max: 10, default: 1 }
"#;

    /// A [`DevicePins`] over a flat word map, so a test can drive an "MCU
    /// output" and read back what the device drove without a whole bus.
    #[derive(Default)]
    struct FakePads {
        words: BTreeMap<u64, u32>,
    }

    impl DevicePins for FakePads {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.words.get(&addr).map(|v| (v >> bit) & 1 != 0)
        }
        fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
            let v = self.words.entry(addr).or_insert(0);
            if high {
                *v |= 1 << bit;
            } else {
                *v &= !(1 << bit);
            }
        }
        fn drive_input_bit(&mut self, _addr: u64, _bit: u8, _high: bool) -> bool {
            false
        }
    }

    fn device() -> (DeclarativeGpioDevice, FakePads) {
        let desc = DeviceDescriptor::from_yaml(FIXTURE).expect("fixture parses");
        validate_descriptor(&desc).expect("fixture validates");
        const CH: &[InputChannel] = &[InputChannel {
            key: "weight",
            label: "W",
            unit: "g",
            min: 0.0,
            max: 10.0,
        }];
        let dev = DeclarativeGpioDevice::new(
            "scale".into(),
            &desc,
            vec![BoundPin {
                role: "CLK".into(),
                addr: 0x1000,
                bit: 3,
            }],
            vec![BoundPin {
                role: "OUT".into(),
                addr: 0x2000,
                bit: 5,
            }],
            8_000_000,
            CH,
        )
        .expect("constructs");
        (dev, FakePads::default())
    }

    #[test]
    fn an_mcu_edge_reaches_a_rule_and_the_answer_reaches_a_pad() {
        let (mut dev, mut pads) = device();
        // Anchor: the first pass only samples.
        dev.service(&mut pads, 0);
        assert_eq!(pads.output_bit(0x2000, 5), None, "nothing driven yet");

        pads.drive_idr_bit(0x1000, 3, true); // the MCU raises CLK
        dev.service(&mut pads, 1);
        assert_eq!(dev.rule_machine().var("n"), 1);
        assert_eq!(pads.output_bit(0x2000, 5), Some(true), "OUT followed CLK");

        pads.drive_idr_bit(0x1000, 3, false);
        dev.service(&mut pads, 2);
        assert_eq!(pads.output_bit(0x2000, 5), Some(false));
        assert_eq!(
            dev.rule_machine().var("n"),
            1,
            "a falling edge is not a rise"
        );
    }

    #[test]
    fn the_derived_clock_fires_timers_from_cycles() {
        let (mut dev, mut pads) = device();
        dev.service(&mut pads, 0);
        // 8 MHz ⇒ 8 cycles per µs; a 100 µs period is 800 cycles.
        dev.service(&mut pads, 799);
        assert_eq!(dev.rule_machine().var("n"), 0, "inside the first period");
        dev.service(&mut pads, 800);
        assert_eq!(dev.rule_machine().var("n"), 100, "the timer came due");
    }

    #[test]
    fn a_sub_microsecond_tick_does_not_lose_time() {
        let (mut dev, mut pads) = device();
        dev.service(&mut pads, 0);
        // One cycle at a time: 8 cycles = 1 µs. Without the remainder each
        // call would convert to 0 µs and the clock would never move at all.
        for c in 1..=800 {
            dev.service(&mut pads, c);
        }
        assert_eq!(dev.rule_machine().var("n"), 100);
    }

    #[test]
    fn a_descriptor_with_no_rules_is_refused() {
        let desc = DeviceDescriptor::from_yaml(
            "type: t\nbehavior:\n  primitive: gpio_device\n  pins: { A: a_pin }\n",
        )
        .unwrap();
        let err = validate_descriptor(&desc).unwrap_err();
        assert!(err.to_string().contains("no `rules:`"), "{err}");
    }

    #[test]
    fn a_rule_naming_an_undeclared_pin_is_a_load_error() {
        let desc = DeviceDescriptor::from_yaml(
            r#"
type: t
behavior:
  primitive: gpio_device
  pins: { A: a_pin }
  rules:
    - on: { pin: A, edge: rising }
      do: [ { pin: NOPE, level: 1 } ]
"#,
        )
        .unwrap();
        let err = validate_descriptor(&desc).unwrap_err();
        assert!(format!("{err:#}").contains("NOPE"), "{err:#}");
    }

    #[test]
    fn a_malformed_guard_is_a_load_error_naming_the_rule() {
        let desc = DeviceDescriptor::from_yaml(
            r#"
type: t
behavior:
  primitive: gpio_device
  pins: { A: a_pin }
  outputs: [B]
  rules:
    - on: { pin: A, edge: rising }
      when: "reg(X) $"
      do: [ { pin: B, level: 1 } ]
"#,
        )
        .unwrap();
        let err = validate_descriptor(&desc).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("rules[0].when"), "{text}");
    }
}
