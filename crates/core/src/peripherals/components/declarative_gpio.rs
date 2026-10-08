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
//! resident service pass and reaches its pads through the narrowed [`DevicePins`]
//! port. Finite schedules also expose their next deadline through the resident
//! interface, independently of host sampling and timer advancement.
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
//! Scheduled GPIO behavior
//! =======================
//! GPIO rules also express stimulus-driven phase walks with optional
//! cycle-quantized timers. Finite exact or peripheral-grid edge schedules encode
//! DHT response frames and HC-SR04 echo windows without part-specific runtime code.

use std::collections::BTreeMap;

use anyhow::{anyhow, Context, Result};
use labwired_config::{DeviceDescriptor, Event, PinEdge};

use super::declarative_artifact::CompiledArtifact;
use super::declarative_regs::{apply_timing_action, TimerBank};
use super::gpio_schedule::ScheduleBank;
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
    grid_eligible: bool,
    seed_input_clamp: bool,
    schedules: BTreeMap<String, labwired_config::GpioScheduleSpec>,
    schedule_bank: ScheduleBank,
    schedule_fault: Option<String>,
    pin_sampling: BTreeMap<String, labwired_config::PinSampling>,
    observed_since: Vec<Option<u64>>,
    preceding_hold: Vec<Option<u64>>,
    sensor_levels: BTreeMap<String, bool>,
    expr_precision: BTreeMap<String, labwired_config::InputPrecision>,
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
    /// Declared fallback for unreadable pads, resolved once at construction.
    observed_defaults: Vec<bool>,
    /// Last physical output levels, distinct from queued rule transitions.
    last_driven: Vec<Option<bool>>,
    settle_outputs: bool,
    /// Which pads moved in the store currently being serviced. Written in
    /// phase 1 and consumed in phase 3, so the per-pad events are raised from
    /// the SAME snapshot the simultaneous-pad event saw rather than from a
    /// second read of the pads.
    last_changed: Vec<bool>,
    /// Pads this device drives (IDR + the external-world seam).
    driven: Vec<BoundPin>,
    /// Measurement slots in engineering units, keyed by input-channel key.
    slots: BTreeMap<String, f64>,
    /// Per-channel `expr_scale` — the counts per engineering unit a rule's
    /// `input()` sees. See [`labwired_config::InputSpec::expr_scale`].
    expr_scale: BTreeMap<String, f64>,
    channels: std::borrow::Cow<'static, [InputChannel]>,
    cpu_hz: u64,
    cycle_timers: bool,
    input_timer_start_on_service: bool,
    pending_input_timers: BTreeMap<String, bool>,
    elapsed_cycles: u64,
    /// Simulated cycle at the previous service, for the derived clock.
    last_cycle: Option<u64>,
    /// Remainder of cycles × 1,000,000 divided by cpu_hz. Retaining this
    /// rational numerator prevents drift at fractional and sub-MHz clocks.
    cycle_remainder: u64,
    /// Device time in µs, derived from cycles above.
    elapsed_us: u64,
    /// What this part SHOWS, declared rather than coded. `None` for a part that
    /// shows nothing, which is every `gpio_device` written before the
    /// `artifact:` key existed. See
    /// [`CompiledArtifact`](super::declarative_artifact::CompiledArtifact).
    artifact: Option<CompiledArtifact>,
    /// Whether any rule listens for the simultaneous-pad event, so the sampler
    /// skips building the changed-pad list when nothing would read it.
    listens_for_pin_sets: bool,
    /// Scratch for the pads that moved in the store being serviced, reused
    /// across passes so the write path allocates nothing per store.
    moved: Vec<String>,
    /// **Whether the module's supply pins are connected in the design.**
    ///
    /// The same key, the same asymmetry and the same reason as on the SPI
    /// primitive — ABSENT MEANS POWERED. A TM1637 module wired SCK/DIO and
    /// nothing else decodes perfectly here and is dark on a bench; the gate is
    /// in [`Self::service`], so an unpowered part observes no pad, runs no rule
    /// and drives no output, and its RAM stays at its power-on value BY
    /// CONSTRUCTION rather than being blanked at readback.
    powered: bool,
    /// Output-register addresses this device must be serviced on SYNCHRONOUSLY,
    /// from the MMIO write hook rather than the peripheral tick. Non-empty when
    /// a rule listens for a pin EDGE — see
    /// [`BusResidentDevice::edge_service_addrs`].
    edge_addrs: Vec<u64>,
}

impl DeclarativeGpioDevice {
    /// Build from a descriptor whose pins have already been resolved to pads.
    pub fn new(
        id: String,
        descriptor: &DeviceDescriptor,
        observed: Vec<BoundPin>,
        driven: Vec<BoundPin>,
        cpu_hz: u64,
        channels: std::borrow::Cow<'static, [InputChannel]>,
    ) -> Result<Self> {
        let mut machine = RuleMachine::from_behavior(&descriptor.behavior)?.ok_or_else(|| {
            anyhow!(
                "gpio_device '{}' declares no rules, timers or outputs — a pins-only part with \
                 no behaviour would attach, answer nothing, and look like a working device",
                descriptor.r#type
            )
        })?;
        let mut slots = BTreeMap::new();
        let mut expr_scale = BTreeMap::new();
        let mut expr_precision = BTreeMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
                expr_precision.insert(input.key.clone(), input.expr_precision);
                if let Some(scale) = input.expr_scale {
                    expr_scale.insert(input.key.clone(), scale);
                }
            }
        }
        // A part whose rules listen for an EDGE is clocked by firmware, not by
        // the tick: it must be serviced from the write hook or it samples pad
        // LEVELS and misses the transitions between them. A part whose rules
        // only listen for timers and stimuli stays tick-driven and costs the
        // write path the `is_empty()` check and nothing else.
        // ⚠️ BOTH pad events count. A part whose ONLY pad rule is
        // `on: { pins: [...] }` is every bit as firmware-clocked as one using
        // `on: { pin: X }`; leaving it off this list would leave it serviced
        // only on the tick, which for a bit-banged protocol delivers one edge
        // per tick interval or none at all.
        let edge_driven = descriptor
            .behavior
            .rules
            .iter()
            .any(|r| matches!(r.on, Event::Pin { .. } | Event::Pins { .. }));
        let edge_addrs: Vec<u64> = if edge_driven {
            let mut addrs: Vec<u64> = observed.iter().map(|p| p.addr).collect();
            addrs.sort_unstable();
            addrs.dedup();
            addrs
        } else {
            Vec::new()
        };
        let mut initial_levels = descriptor.behavior.pin_defaults.clone();
        for schedule in descriptor.behavior.schedules.values() {
            initial_levels
                .entry(schedule.output.clone())
                .or_insert(schedule.idle);
        }
        for pin in &driven {
            if let Some(level) = initial_levels.get(&pin.role) {
                machine.drive_pin(&pin.role, *level);
            }
        }
        let cycle_timers =
            descriptor.behavior.timer_clock == labwired_config::GpioTimerClock::CyclesFloor;
        let listens_for_pin_sets = machine.listens_for_pin_sets();
        Ok(Self {
            id,
            powered: true,
            artifact: CompiledArtifact::from_descriptor(descriptor)?,
            listens_for_pin_sets,
            moved: Vec::new(),
            seed_input_clamp: descriptor.behavior.seed_input_clamp,
            grid_eligible: descriptor.behavior.timers.is_empty()
                && descriptor
                    .behavior
                    .rules
                    .iter()
                    .all(|r| matches!(r.on, Event::Pin { .. } | Event::Pins { .. })),
            schedules: descriptor.behavior.schedules.clone(),
            schedule_bank: ScheduleBank::default(),
            schedule_fault: None,
            pin_sampling: descriptor.behavior.pin_sampling.clone(),
            observed_since: vec![None; observed.len()],
            preceding_hold: vec![None; observed.len()],
            sensor_levels: initial_levels,
            expr_precision,
            machine,
            timers: if cycle_timers {
                TimerBank::new_cycles(&descriptor.behavior.timers, cpu_hz)
            } else {
                TimerBank::new(&descriptor.behavior.timers)
            },
            cycle_timers,
            input_timer_start_on_service: descriptor.behavior.input_timer_start_on_service,
            pending_input_timers: BTreeMap::new(),
            elapsed_cycles: 0,
            last_seen: observed
                .iter()
                .map(|p| {
                    if !descriptor.behavior.schedules.is_empty()
                        || descriptor.behavior.pin_sampling.get(&p.role)
                            == Some(&labwired_config::PinSampling::OpenDrainRelease)
                    {
                        Some(
                            descriptor
                                .behavior
                                .pin_defaults
                                .get(&p.role)
                                .copied()
                                .unwrap_or(false),
                        )
                    } else {
                        None
                    }
                })
                .collect(),
            observed_defaults: observed
                .iter()
                .map(|p| {
                    descriptor
                        .behavior
                        .pin_defaults
                        .get(&p.role)
                        .or_else(|| {
                            p.role
                                .split_once('[')
                                .and_then(|(group, _)| descriptor.behavior.pin_defaults.get(group))
                        })
                        .copied()
                        .unwrap_or(false)
                })
                .collect(),
            last_driven: vec![None; driven.len()],
            settle_outputs: descriptor.behavior.output_update
                == labwired_config::GpioOutputUpdate::FinalLevel,
            last_changed: vec![false; observed.len()],
            observed,
            driven,
            edge_addrs,
            slots,
            expr_scale,
            channels,
            cpu_hz: cpu_hz.max(1),
            last_cycle: None,
            cycle_remainder: 0,
            elapsed_us: 0,
        })
    }

    /// Declare whether the module's supply is connected. Only ever called with
    /// `false`, from attach, when the compiled manifest explicitly says the
    /// supply pins are on no net. See the `powered` field.
    pub fn with_powered(mut self, powered: bool) -> Self {
        self.powered = powered;
        self
    }

    /// True when the module has a supply. See the `powered` field.
    pub fn powered(&self) -> bool {
        self.powered
    }

    /// Seed a measurement slot from a `config:` override, like every other
    /// declarative primitive does.
    pub fn seed_input(&mut self, key: &str, value: f64) {
        if self.channels.iter().any(|c| c.key == key) {
            let channel = self.channels.iter().find(|c| c.key == key).unwrap();
            self.slots.insert(
                key.to_string(),
                if self.seed_input_clamp {
                    value.clamp(channel.min, channel.max)
                } else {
                    value
                },
            );
        }
    }

    pub fn input_value(&self, key: &str) -> Option<f64> {
        self.slots.get(key).copied()
    }
    pub fn cpu_hz(&self) -> u64 {
        self.cpu_hz
    }
    /// Most recent rejected schedule emission, retaining the preceding waveform.
    pub fn schedule_fault(&self) -> Option<&str> {
        self.schedule_fault.as_deref()
    }

    fn drain_schedule_requests(&mut self, now: u64) {
        for request in self.machine.take_schedule_requests() {
            match request {
                super::rule_machine::ScheduleRequest::Emit {
                    name,
                    values,
                    inputs,
                } => {
                    if let Some(spec) = self.schedules.get(&name) {
                        // Invalid dynamic values leave the previous generation intact.
                        self.schedule_fault = self
                            .schedule_bank
                            .emit(&name, spec, &values, &inputs, self.cpu_hz, now)
                            .err()
                            .map(|error| format!("schedule '{name}': {error:#}"));
                    }
                }
                super::rule_machine::ScheduleRequest::Cancel { name } => {
                    if let Some(spec) = self.schedules.get(&name) {
                        self.schedule_bank.cancel(&name, spec, now);
                    }
                }
            }
        }
    }

    fn drive_output(&mut self, pins: &mut dyn DevicePins, role: &str, sensor_level: bool) {
        self.sensor_levels.insert(role.to_string(), sensor_level);
        let Some(i) = self.driven.iter().position(|p| p.role == role) else {
            return;
        };
        let mut level = sensor_level;
        if self.pin_sampling.get(role) == Some(&labwired_config::PinSampling::OpenDrainRelease) {
            if let Some(j) = self.observed.iter().position(|p| p.role == role) {
                level &= self.last_seen[j].unwrap_or(self.observed_defaults[j]);
            }
        }
        if self.last_driven[i] == Some(level) {
            return;
        }
        self.last_driven[i] = Some(level);
        let pin = &self.driven[i];
        let _ = pins.drive_input_bit(pin.addr, pin.bit, level);
        pins.drive_idr_bit(pin.addr, pin.bit, level);
    }

    /// Read-only view of the rule machine, for tests and diagnostics.
    pub fn rule_machine(&self) -> &RuleMachine {
        &self.machine
    }

    fn fire(&mut self, event: Event) {
        let mut ctx = PinOnlyCtx {
            slots: &mut self.slots,
            expr_scale: &self.expr_scale,
            expr_precision: &self.expr_precision,
        };
        self.machine.fire(&event, 0, &mut ctx);
    }

    /// Derive rational microseconds without truncating fractional MHz clocks.
    fn advance_clock(&mut self, now: u64) {
        let delta = self
            .last_cycle
            .replace(now)
            .map_or(0, |previous| now.saturating_sub(previous));
        self.elapsed_cycles = self.elapsed_cycles.saturating_add(delta);
        let total = self.cycle_remainder as u128 + delta as u128 * 1_000_000;
        let us = (total / self.cpu_hz as u128).min(u64::MAX as u128) as u64;
        self.cycle_remainder = (total % self.cpu_hz as u128) as u64;
        self.elapsed_us = self.elapsed_us.saturating_add(us);
        self.machine.advance_time_us(us);
        // Input has no current-cycle argument. Delay these requests until the
        // clock reaches this serviced tick, before considering old deadlines.
        let timer_now = if self.cycle_timers {
            self.elapsed_cycles
        } else {
            self.elapsed_us
        };
        for (name, start) in std::mem::take(&mut self.pending_input_timers) {
            if start {
                self.timers.start_named(&name, timer_now);
            } else {
                self.timers.stop_named(&name);
            }
        }
        if self.cycle_timers {
            // Bounded replay, retaining overdue deadlines for the next pass.
            // Rules may stop a completed walk immediately, before another due
            // event is generated; large idle gaps therefore cost no work.
            for _ in 0..65_536 {
                let Some((name, _)) = self.timers.pop_due(timer_now) else {
                    break;
                };
                self.fire(Event::Timer { name });
                self.drain_timer_requests();
            }
        } else if us != 0 && !self.timers.is_empty() {
            let mut registers = std::collections::HashMap::new();
            for (name, actions) in self.timers.due_by_timer(self.elapsed_us) {
                for action in &actions {
                    apply_timing_action(action, &mut registers);
                }
                self.fire(Event::Timer { name });
            }
            self.drain_timer_requests();
        }
    }

    /// Apply whatever `timer:` actions the rules queued to the bank.
    fn drain_timer_requests(&mut self) {
        let requests = self.machine.take_timer_requests();
        for (name, start) in requests {
            if start {
                self.timers.start_named(
                    &name,
                    if self.cycle_timers {
                        self.elapsed_cycles
                    } else {
                        self.elapsed_us
                    },
                );
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
        // THE SUPPLY GATE. One return, ahead of every phase: no sampling, no
        // events, no clock, no pad drive. Placing it here rather than on the
        // artifact is what makes an unpowered part's RAM stay at its power-on
        // value instead of being blanked at report time — the same argument the
        // hand-written MAX7219 made for putting its gate in `transfer`.
        if !self.powered {
            return;
        }
        // ── PHASE 1: resample EVERY observed pad, raise nothing ────────────
        //
        // ⚠️ THE TWO PHASES ARE THE WHOLE SIMULTANEOUS-PAD FIX, AND THE ORDER
        // IS THE ARGUMENT. One MMIO store can move several pads at once — a
        // BSRR write sets CLK and clears DIO in a single instruction — and this
        // pass is called ONCE for that store. Sampling a pad and raising its
        // event before the next pad has been sampled hands the second rule a
        // STALE level for the first: a TM1637 START is "DIO fell while CLK was
        // high", and decomposed that way a store that moves both lines either
        // synthesises a START that never happened or misses one that did.
        //
        // So: the whole snapshot is installed in the machine first, and only
        // then is anything raised. Inside any rule, `pin(X)` is the level pad X
        // holds AFTER the store, for every X.
        self.moved.clear();
        let mut changed_any = false;
        for i in 0..self.observed.len() {
            // An address that does not read back means the MCU is driving
            // nothing there; a declared fallback represents its pull-up/down.
            // Existing descriptors keep their default LOW.
            let pin = &self.observed[i];
            let level = if self.pin_sampling.get(&pin.role)
                == Some(&labwired_config::PinSampling::OpenDrainRelease)
            {
                pins.released_output_bit(pin.addr, pin.bit)
            } else {
                pins.output_bit(pin.addr, pin.bit)
            }
            .unwrap_or(self.observed_defaults[i]);
            let was = self.last_seen[i].replace(level);
            self.preceding_hold[i] = None;
            if was.is_some() && was != Some(level) {
                self.preceding_hold[i] =
                    self.observed_since[i].map(|start| now.saturating_sub(start));
                self.observed_since[i] = Some(now);
            }
            self.machine
                .set_observed_level(&self.observed[i].role, level);
            // ⚠️ THE TWO PAD EVENTS DIFFER ON THE FIRST SAMPLE, AND THEY MUST.
            //
            // `on: { pin: X, edge: … }` is an EDGE event: it needs a previous
            // level, and the first pass has none, so it raises nothing. That is
            // the behaviour every part written against it depends on.
            //
            // `on: { pins: [...] }` is a LEVEL event — "these pads now hold
            // these levels" — and the first store is as much a statement of
            // levels as any later one. Skipping it would mean a combinational
            // part shows NOTHING until the second store: firmware that lights a
            // digit with one `BSRR` write and then leaves it alone would leave
            // the panel blank forever. It would also cost the TM1637 its first
            // START, because its idle-high seed IS a previous level, stated in
            // the descriptor rather than discovered from a pad.
            if self.listens_for_pin_sets && was != Some(level) {
                self.moved.push(self.observed[i].role.clone());
            }
            if was.is_none() {
                continue;
            }
            if was != Some(level) {
                changed_any = true;
            }
            self.last_changed[i] = was != Some(level);
        }

        // ── PHASE 2: the simultaneous-pad event, ONCE for the whole store ──
        //
        // Raised before the per-pad events so a part framed by two lines
        // decodes the store as one thing, and a part clocked on one line is
        // untouched.
        if self.listens_for_pin_sets && !self.moved.is_empty() {
            let names = std::mem::take(&mut self.moved);
            self.fire(Event::Pins {
                names: names.clone(),
            });
            self.drain_timer_requests();
            self.moved = names;
        }

        // ── PHASE 3: the per-pad edges, in observed order ──────────────────
        if changed_any {
            for i in 0..self.observed.len() {
                if !self.last_changed[i] {
                    continue;
                }
                self.last_changed[i] = false;
                let level = self.last_seen[i].unwrap_or(false);
                let event = Event::Pin {
                    name: self.observed[i].role.clone(),
                    edge: if level {
                        PinEdge::Rising
                    } else {
                        PinEdge::Falling
                    },
                };
                if let Some(held) = self.preceding_hold[i] {
                    let mut ctx = PinOnlyCtx {
                        slots: &mut self.slots,
                        expr_scale: &self.expr_scale,
                        expr_precision: &self.expr_precision,
                    };
                    self.machine
                        .fire_pin(&event, 0, held, self.cpu_hz, &mut ctx);
                } else {
                    self.fire(event);
                }
                // An edge rule may have started or stopped a timer.
                self.drain_timer_requests();
            }
        }

        self.advance_clock(now);
        self.drain_schedule_requests(now);

        let pending = self.machine.take_pin_drives();
        // Combinational parts may settle several host/row events before one
        // service. Protocol parts retain every transition and its action order.
        let pending = if self.settle_outputs {
            self.driven
                .iter()
                .filter_map(|pin| {
                    pending
                        .iter()
                        .rev()
                        .find(|(role, _)| *role == pin.role)
                        .cloned()
                })
                .collect()
        } else {
            pending
        };
        for (role, level) in pending {
            self.drive_output(pins, &role, level);
        }
        let interval = pins.peripheral_tick_interval();
        self.service_scheduled_edges(pins, now, interval);
        let composed: Vec<_> = self
            .pin_sampling
            .iter()
            .filter(|(_, sampling)| **sampling == labwired_config::PinSampling::OpenDrainRelease)
            .map(|(role, _)| {
                (
                    role.clone(),
                    self.sensor_levels.get(role).copied().unwrap_or(true),
                )
            })
            .collect();
        for (role, level) in composed {
            self.drive_output(pins, &role, level);
        }
    }

    fn next_edge_deadline_cycle(&self, _now: u64, interval: u64) -> Option<u64> {
        if self.powered {
            self.schedule_bank.next_deadline(interval)
        } else {
            None
        }
    }
    fn service_scheduled_edges(&mut self, pins: &mut dyn DevicePins, now: u64, interval: u64) {
        if !self.powered {
            return;
        }
        for (role, level) in self.schedule_bank.service(now, interval) {
            self.machine.set_scheduled_pin_level(&role, level);
            self.drive_output(pins, &role, level);
        }
    }
    fn schedule_revision(&self) -> u64 {
        self.schedule_bank.revision()
    }
    fn has_grid_schedules(&self) -> bool {
        !self.schedules.is_empty()
            && self
                .schedules
                .values()
                .all(|s| s.timing == labwired_config::ScheduleTiming::PeripheralTickGrid)
    }

    /// ⚠️ STATED, not inherited, and the two halves of the condition are each a
    /// fact a default would get wrong.
    ///
    /// A part that owns NO TIMER and DRIVES NO PAD can only change when
    /// firmware stores to an output register, and `edge_service_addrs` already
    /// services it synchronously inside that store. A tick pass would resample
    /// pads that cannot have moved. The TM1637 and the bare 7-segment digit are
    /// both this, and their hand-written predecessors each said so by hand —
    /// left at the trait's `true`, a board carrying either would force
    /// `requires_cycle_accurate()` and pin `max_safe_tick_interval()` to 1: a
    /// performance regression against the models they replace, invisible to
    /// every behavioural test.
    ///
    /// ⚠️ A `gpio_device` with `timers:` MUST NOT say false — its clock only
    /// advances on the tick — and one that DRIVES a pad must not either, since
    /// the level it answers with is put on the pad by this same pass. The HX711
    /// is both, and answers `true` through this expression rather than through
    /// an override somebody has to remember.
    // Grid-only finite schedules are serviced by resident deadlines when every
    // other action is synchronous with a host pad write. The bus separately
    // gates batching on its scheduler, timer, and live-cycle capabilities.
    fn needs_per_cycle_service(&self) -> bool {
        !self.timers.is_empty()
            || !(self.driven.is_empty() || self.has_grid_schedules() && self.grid_eligible)
    }

    /// No timers and only pad-edge rules (`grid_eligible`) means every change
    /// this part makes on its own is a schedule edge, and every pad edge is
    /// serviced from the write hook. So it is idle exactly when no schedule
    /// edge is pending: a DHT22 between reads, not during one.
    fn idle_skip_safe(&self) -> bool {
        !self.needs_per_cycle_service()
            || (self.grid_eligible && self.schedule_bank.next_deadline(1).is_none())
    }

    fn edge_service_addrs(&self) -> &[u64] {
        &self.edge_addrs
    }

    /// A pins-only part has no controller trait to hang evidence on — it binds
    /// on PADS — so #1176 gave [`BusResidentDevice`] the seam directly. This is
    /// what fills it for a DESCRIPTOR: a part that declares an `artifact:`
    /// reports through the same door the hand-written models used, and a part
    /// that declares none answers `None` rather than an empty panel.
    fn evidence(&self) -> Option<&dyn crate::inspect::DeviceEvidence> {
        self.artifact.as_ref().map(|_| self as _)
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

impl crate::inspect::DeviceEvidence for DeclarativeGpioDevice {
    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let Some(artifact) = &self.artifact else {
            return Vec::new();
        };
        // The renderer needs a `RuleCtx` to evaluate a `meta.value:` expression
        // that reads `input()`. Building it here needs `&mut` slots, which an
        // evidence read does not have — so it reads a CLONE of the slots. That
        // is sound because rendering never writes: `set_input` on this context
        // would change a clone nobody keeps, and the only way to reach it is an
        // action, which a `meta.value:` expression cannot contain.
        let mut slots = self.slots.clone();
        let ctx = PinOnlyCtx {
            slots: &mut slots,
            expr_scale: &self.expr_scale,
            expr_precision: &self.expr_precision,
        };
        let rendered = artifact.render(&self.machine, &ctx, id, opts);
        // Published, never swallowed, and stamped with WHY it is blank.
        vec![if self.powered {
            rendered
        } else {
            super::supply::mark_unpowered(rendered)
        }]
    }
}

impl SimInput for DeclarativeGpioDevice {
    fn input_channels(&self) -> &[InputChannel] {
        &self.channels
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.slots.insert(key.to_string(), value);
        self.fire(Event::Input {
            key: key.to_string(),
        });
        if self.input_timer_start_on_service {
            self.pending_input_timers
                .extend(self.machine.take_timer_requests());
        } else {
            self.drain_timer_requests();
        }
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
    let mut seen = std::collections::BTreeSet::new();
    for (role, binding) in &b.pins {
        let valid = match binding {
            labwired_config::PinBinding::Scalar(key) => !key.trim().is_empty(),
            labwired_config::PinBinding::List(keys) => {
                (1..=labwired_config::MAX_PIN_GROUP_SIZE).contains(&keys.len())
                    && keys.iter().all(|key| !key.trim().is_empty())
            }
            labwired_config::PinBinding::ConfigList { config, count } => {
                !config.trim().is_empty()
                    && (1..=labwired_config::MAX_PIN_GROUP_SIZE).contains(count)
            }
        };
        anyhow::ensure!(
            valid,
            "gpio_device '{}' has an empty list or blank config key for pin role '{role}'",
            desc.r#type
        );
        for name in binding.names(role) {
            anyhow::ensure!(
                seen.insert(name.clone()),
                "gpio_device '{}' duplicates pin role '{name}'",
                desc.r#type
            );
        }
    }
    for role in b.pin_defaults.keys() {
        anyhow::ensure!(
            b.pins.contains_key(role) || seen.contains(role) || b.outputs.contains(role),
            "gpio_device '{}' pin_defaults names undeclared role '{role}'",
            desc.r#type
        );
    }
    let mut config_keys = std::collections::BTreeSet::new();
    for binding in b.pins.values() {
        match binding {
            labwired_config::PinBinding::Scalar(key) => {
                config_keys.insert(key.as_str());
            }
            labwired_config::PinBinding::List(keys) => {
                config_keys.extend(keys.iter().map(String::as_str));
            }
            // A list-valued placement needs a list, not one default pad label.
            labwired_config::PinBinding::ConfigList { .. } => {}
        }
    }
    for role in &b.outputs {
        config_keys.insert(b.output_pins.get(role).unwrap_or(role).as_str());
    }
    for (key, label) in &b.pin_config_defaults {
        anyhow::ensure!(
            !key.trim().is_empty() && config_keys.contains(key.as_str()) && !label.trim().is_empty(),
            "gpio_device '{}' pin_config_defaults has an undeclared/blank config key or blank pad label: '{key}'",
            desc.r#type
        );
    }
    for (name, spec) in &b.schedules {
        super::gpio_schedule::validate(spec).with_context(|| format!("schedule '{name}'"))?;
        anyhow::ensure!(
            b.outputs.contains(&spec.output),
            "schedule '{name}' names undeclared output"
        );
        let inputs: Vec<_> = desc
            .metadata
            .iter()
            .flat_map(|m| m.inputs.iter().map(|i| i.key.as_str()))
            .collect();
        for segment in &spec.segments {
            let holds: Vec<_> = match segment {
                labwired_config::ScheduleSegment::Hold { hold } => vec![hold],
                labwired_config::ScheduleSegment::Bits { bits } => {
                    let expr = labwired_config::expr::Expr::parse(&bits.value)?;
                    let mut names = Vec::new();
                    expr.input_names(&mut names);
                    for key in names {
                        anyhow::ensure!(
                            inputs.contains(&key.as_str()),
                            "schedule '{name}' references undeclared input '{key}'"
                        );
                    }
                    let mut names = Vec::new();
                    expr.var_names(&mut names);
                    for key in names {
                        anyhow::ensure!(
                            b.vars.contains_key(&key),
                            "schedule '{name}' references undeclared var '{key}'"
                        );
                    }
                    bits.zero.iter().chain(&bits.one).collect()
                }
            };
            for hold in holds {
                if let labwired_config::ScheduleDuration::InputLinear { input, .. } = &hold.us {
                    anyhow::ensure!(
                        inputs.contains(&input.as_str()),
                        "schedule '{name}' references undeclared input '{input}'"
                    );
                }
            }
        }
    }
    for role in b.pin_sampling.keys() {
        anyhow::ensure!(
            seen.contains(role),
            "pin_sampling names undeclared observed pin '{role}'"
        );
    }
    // Compiling the expressions here is the whole point of preflight: a
    // malformed guard must be a load error naming the rule, not a surprise at
    // the first edge.
    labwired_config::compile_rules(&b.rules)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("gpio_device '{}' has an invalid rule", desc.r#type))?;
    validate_rule_names(desc)?;
    // Same reason the rules are compiled here: a malformed artifact must be a
    // LOAD error naming the part, not a panel that renders nothing at the first
    // inspect.
    if let Some(artifact) = &b.artifact {
        artifact.validate(&desc.r#type)?;
    }
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
    let pins: Vec<String> = b.pin_names();
    let inputs: Vec<String> = desc
        .metadata
        .as_ref()
        .map(|m| m.inputs.iter().map(|i| i.key.clone()).collect())
        .unwrap_or_default();
    let mut rules = b.rules.clone();
    for spec in b.schedules.values() {
        for segment in &spec.segments {
            if let labwired_config::ScheduleSegment::Bits { bits } = segment {
                rules.push(labwired_config::Rule {
                    on: Event::Start,
                    min_hold_us: None,
                    when: Some(bits.value.clone()),
                    actions: Vec::new(),
                });
            }
        }
    }
    labwired_config::validate_rule_names(
        &rules,
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
            // Shared by the gpio, I²C and SPI primitives: a `frame_byte(N)` is
            // checked against the part's OWN `frames:` block, so a pins-only
            // part reading one is a load error and a framed part's index is
            // bounded by the declared length.
            frames: b.frames.as_ref(),
        },
    )
    .with_context(|| format!("part '{}' names something it does not declare", desc.r#type))
}

// ─── the kit wrapper ───────────────────────────────────────────────────────

/// A `gpio_device` descriptor as a [`PeripheralKit`], so a ported bit-banged
/// part keeps its entry in the peripheral MANIFEST.
///
/// Attach itself still goes through `SystemBus::attach_declarative_device` —
/// this adds no second attach path, it adds the metadata one. Without it a part
/// that becomes a `configs/devices/*.yaml` descriptor still attaches (the
/// universal resolver's declarative step finds it) but VANISHES from the
/// library: its label, its `config:` keys and its stimulus channels are gone
/// from the manifest the browser reads, and nothing fails. That is how `keypad`,
/// `dht22` and `rotary_encoder` came to be absent from it.
pub struct DeclarativeGpioKit {
    descriptor: DeviceDescriptor,
    channels: std::borrow::Cow<'static, [InputChannel]>,
    metadata: crate::peripherals::kit::KitMetadata,
}

impl std::fmt::Debug for DeclarativeGpioKit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarativeGpioKit")
            .field("type", &self.descriptor.r#type)
            .finish()
    }
}

impl DeclarativeGpioKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;
        let channels = super::declarative_i2c::owned_channels(&descriptor);
        let metadata = super::declarative_i2c::owned_gpio_metadata(&descriptor, channels.clone());
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }
}

impl crate::peripherals::kit::PeripheralKit for DeclarativeGpioKit {
    fn metadata(&self) -> &crate::peripherals::kit::KitMetadata {
        &self.metadata
    }

    fn attach(&self, ctx: &mut crate::peripherals::kit::AttachCtx<'_>) -> Result<()> {
        // ONE attach path: the same call the universal resolver's declarative
        // step makes. A kit that built the device itself would be a second
        // implementation of pad binding, which is exactly the drift this
        // primitive exists to remove.
        let _ = self.channels;
        ctx.bus.attach_declarative_device(ctx.ext, &self.descriptor)
    }
}

/// Same bridge the I²C kits use: the registry is a `const` slice of
/// `&'static dyn PeripheralKit`, and a descriptor is parsed at runtime.
impl crate::peripherals::kit::PeripheralKit for std::sync::LazyLock<DeclarativeGpioKit> {
    fn metadata(&self) -> &crate::peripherals::kit::KitMetadata {
        std::sync::LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut crate::peripherals::kit::AttachCtx<'_>) -> Result<()> {
        std::sync::LazyLock::force(self).attach(ctx)
    }
}

/// Avia HX711 24-bit load-cell ADC (declarative `hx711.yaml`).
///
/// Migrated from the hand-written `components/hx711.rs`, which is DELETED
/// along with its private `SystemBus::hx711` list and the `maybe_clock_hx711`
/// write hook — that hook is now the generic
/// [`BusResidentDevice::edge_service_addrs`] path any descriptor can use.
/// `tests/hx711_migration_parity.rs` pins the protocol.
pub static HX711_KIT: std::sync::LazyLock<DeclarativeGpioKit> = std::sync::LazyLock::new(|| {
    DeclarativeGpioKit::from_yaml(
        labwired_config::embedded_device_yaml("hx711").expect("hx711 descriptor is embedded"),
    )
    .expect("hx711.yaml is a valid declarative gpio descriptor")
});

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
            key: std::borrow::Cow::Borrowed("weight"),
            label: std::borrow::Cow::Borrowed("W"),
            unit: std::borrow::Cow::Borrowed("g"),
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
            CH.into(),
        )
        .expect("constructs");
        (dev, FakePads::default())
    }

    fn sensor(kind: &str, hz: u64) -> (DeclarativeGpioDevice, FakePads) {
        let desc = DeviceDescriptor::embedded(kind).unwrap().unwrap();
        validate_descriptor(&desc).unwrap();
        let data = kind.starts_with("dht");
        let dev = DeclarativeGpioDevice::new(
            kind.into(),
            &desc,
            vec![BoundPin {
                role: if data { "DATA" } else { "TRIG" }.into(),
                addr: 1,
                bit: 0,
            }],
            vec![BoundPin {
                role: if data { "DATA" } else { "ECHO" }.into(),
                addr: 2,
                bit: 0,
            }],
            hz,
            super::super::declarative_i2c::owned_channels(&desc),
        )
        .unwrap();
        (dev, FakePads::default())
    }

    #[test]
    fn declarative_dht_matches_oracle_for_every_cycle_and_latches_inputs() {
        for kind in ["dht22", "dht11"] {
            for (t, h) in [
                (22.0, 50.0),
                (-0.01, 65.3),
                (-0.0, 65.3),
                (-1e-50, 65.3),
                (-12.35, 89.95),
                (79.95, 99.95),
                (-100.0, 120.0),
                (100.0, -20.0),
            ] {
                for hz in [1_000_000, 1_500_001, 32_768] {
                    let (mut dev, mut pads) = sensor(kind, hz);
                    dev.seed_input("temperature", t);
                    dev.seed_input("humidity", h);
                    let mut oracle = super::super::dht22::Dht22::new_with_frame(
                        "oracle".into(),
                        1,
                        2,
                        0,
                        hz,
                        t as f32,
                        h as f32,
                        kind == "dht11",
                    );
                    pads.drive_idr_bit(1, 0, false);
                    dev.service(&mut pads, 10);
                    oracle.observe_line(false, 10);
                    let release = 10 + (1000.0 * (hz as f64 / 1e6)) as u64;
                    pads.drive_idr_bit(1, 0, true);
                    dev.service(&mut pads, release);
                    oracle.observe_line(true, release);
                    assert_eq!(
                        dev.machine.var("frame") as u64,
                        oracle.armed_frame_bits(),
                        "{kind} {t} {h} {hz}"
                    );
                    dev.set_input("temperature", 42.0).unwrap();
                    for now in release..release + (6000.0 * (hz as f64 / 1e6)) as u64 {
                        dev.service_scheduled_edges(&mut pads, now, 1);
                        assert_eq!(
                            pads.output_bit(2, 0),
                            Some(oracle.pad_high_at(now)),
                            "{kind} {t} {h} {hz} {now}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn dht_is_idle_skip_safe_only_between_frames() {
        let (mut dev, mut pads) = sensor("dht22", 1_000_000);
        assert!(
            dev.needs_per_cycle_service(),
            "still one instruction per batch"
        );
        assert!(dev.idle_skip_safe(), "idle before a read");
        pads.drive_idr_bit(1, 0, false);
        dev.service(&mut pads, 10);
        assert!(dev.idle_skip_safe(), "the start pulse is a host edge");
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 1010);
        assert!(!dev.idle_skip_safe(), "a frame is in flight");
        let mut now = 1010;
        while let Some(at) = dev.next_edge_deadline_cycle(now, 1) {
            now = at;
            dev.service_scheduled_edges(&mut pads, now, 1);
        }
        assert!(now > 4000, "the whole frame ran: {now}");
        assert!(dev.idle_skip_safe(), "idle again after the frame");
    }

    #[test]
    fn invalid_dynamic_emission_reports_fault_and_keeps_old_generation() {
        let (mut dev, mut pads) = sensor("hc-sr04", 1_000_000);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 10);
        let revision = dev.schedule_revision();
        let deadline = dev.next_edge_deadline_cycle(10, 1);
        pads.drive_idr_bit(1, 0, false);
        dev.service(&mut pads, 11);
        // A rule can set an engineering slot outside the external stimulus API.
        dev.slots.insert("distance".into(), -1.0);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 12);
        assert!(dev.schedule_fault().unwrap().contains("nonnegative"));
        assert_eq!(dev.schedule_revision(), revision);
        assert_eq!(dev.next_edge_deadline_cycle(12, 1), deadline);
    }

    #[test]
    fn dht_requires_observed_low_duration_and_falling_aborts() {
        let (mut dev, mut pads) = sensor("dht22", 1_000_000);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 5000);
        assert_eq!(dev.next_edge_deadline_cycle(5000, 1), None);
        pads.drive_idr_bit(1, 0, false);
        dev.service(&mut pads, 6000);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 6999);
        assert_eq!(dev.next_edge_deadline_cycle(6999, 1), None);
        pads.drive_idr_bit(1, 0, false);
        dev.service(&mut pads, 7000);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 8000);
        assert_eq!(dev.next_edge_deadline_cycle(8000, 1), Some(8030));
        dev.service_scheduled_edges(&mut pads, 8030, 1);
        assert_eq!(pads.output_bit(2, 0), Some(false));
        pads.drive_idr_bit(1, 0, false);
        dev.service(&mut pads, 8040);
        assert_eq!(dev.next_edge_deadline_cycle(8040, 1), None);
        assert_eq!(
            pads.output_bit(2, 0),
            Some(false),
            "host low wins over cancelled sensor idle"
        );
    }
    #[test]
    fn hcsr04_retrigger_replaces_active_window_with_new_latched_distance() {
        let (mut dev, mut pads) = sensor("hc-sr04", 1_000_000);
        let mut oracle =
            crate::peripherals::hc_sr04::HcSr04::new("oracle".into(), 1, 0, 2, 0, 1_000_000, 50.0);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 0);
        oracle.observe_trig(true, 0);
        dev.service_scheduled_edges(&mut pads, 250, 1);
        assert_eq!(pads.output_bit(2, 0), Some(true));
        dev.set_input("distance", 2.25).unwrap();
        oracle.set_distance_cm(2.25);
        pads.drive_idr_bit(1, 0, false);
        dev.service(&mut pads, 260);
        oracle.observe_trig(false, 260);
        pads.drive_idr_bit(1, 0, true);
        dev.service(&mut pads, 261);
        oracle.observe_trig(true, 261);
        dev.set_input("distance", 400.0).unwrap();
        for now in 261..700 {
            dev.service_scheduled_edges(&mut pads, now, 1);
            assert_eq!(
                pads.output_bit(2, 0),
                Some(oracle.echo_high_at(now)),
                "{now}"
            );
        }
    }

    #[test]
    fn hcsr04_fractional_width_matches_oracle_and_retrigger_latches() {
        for hz in [1_000_000, 1_500_001, 1000] {
            for distance in [2.1, 50.125, 399.9, -10.0, 1000.0] {
                let (mut dev, mut pads) = sensor("hc-sr04", hz);
                dev.seed_input("distance", distance);
                let mut oracle = crate::peripherals::hc_sr04::HcSr04::new(
                    "oracle".into(),
                    1,
                    0,
                    2,
                    0,
                    hz,
                    distance as f32,
                );
                pads.drive_idr_bit(1, 0, true);
                dev.service(&mut pads, 7);
                oracle.observe_trig(true, 7);
                dev.set_input("distance", 250.0).unwrap();
                for now in 7..7 + (24000.0 * (hz as f64 / 1e6)) as u64 {
                    dev.service_scheduled_edges(&mut pads, now, 1);
                    assert_eq!(
                        pads.output_bit(2, 0),
                        Some(oracle.echo_high_at(now)),
                        "{hz} {distance} {now}"
                    );
                }
            }
        }
    }

    #[test]
    fn scheduled_reply_is_latched_and_serviced_without_sampling_host() {
        let yaml = FIXTURE.replace("  vars: { n: 0 }", "  vars: { n: 0 }\n  schedules:\n    reply:\n      output: OUT\n      idle: false\n      final: false\n      timing: exact\n      segments: [{hold: {level: false, us: 30}}, {hold: {level: true, us: 80}}]")
            .replace("{ var: n, value: \"var(n) + 1\" }, { pin: OUT, level: 1 }", "{ emit_schedule: reply }");
        let desc = DeviceDescriptor::from_yaml(&yaml).unwrap();
        let (old, mut pads) = device();
        let mut dev = DeclarativeGpioDevice::new(
            "schedule".into(),
            &desc,
            old.observed,
            old.driven,
            1_000_000,
            old.channels,
        )
        .unwrap();
        dev.service(&mut pads, 0);
        pads.drive_idr_bit(0x1000, 3, true);
        dev.service(&mut pads, 10);
        assert_eq!(dev.next_edge_deadline_cycle(10, 1), Some(40));
        dev.service_scheduled_edges(&mut pads, 39, 1);
        assert_eq!(pads.output_bit(0x2000, 5), Some(false));
        dev.service_scheduled_edges(&mut pads, 40, 1);
        assert_eq!(pads.output_bit(0x2000, 5), Some(true));
        dev.service_scheduled_edges(&mut pads, 120, 1);
        assert_eq!(pads.output_bit(0x2000, 5), Some(false));
    }

    #[test]
    fn placement_seed_clamping_is_opt_in_for_existing_gpio_descriptors() {
        let (mut dev, _) = device();
        dev.seed_input("weight", 11.0);
        assert_eq!(dev.input_value("weight"), Some(11.0));
        let (mut dht, _) = sensor("dht22", 1_000_000);
        dht.seed_input("temperature", 100.0);
        assert_eq!(dht.input_value("temperature"), Some(80.0));
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
    fn microsecond_timers_keep_fractional_cpu_clock_remainders() {
        for hz in [32_768, 499_999, 1_500_001, 8_000_000] {
            let (mut dev, mut pads) = device();
            dev.cpu_hz = hz;
            dev.service(&mut pads, 0);
            for cycle in 1..=10_000 {
                dev.service(&mut pads, cycle);
                let expected = (cycle * 1_000_000 / hz / 100) * 100;
                assert_eq!(
                    dev.rule_machine().var("n"),
                    expected as i64,
                    "hz={hz}, cycle={cycle}"
                );
            }
        }
    }

    #[test]
    fn pin_config_defaults_reject_undeclared_keys_and_blank_labels() {
        for defaults in ["{ typo: PA0 }", "{ clk_pin: '' }", "{ '': PA0 }"] {
            let yaml = FIXTURE.replace(
                "  outputs: [OUT]",
                &format!("  outputs: [OUT]\n  pin_config_defaults: {defaults}"),
            );
            let desc = DeviceDescriptor::from_yaml(&yaml).unwrap();
            let err = validate_descriptor(&desc).unwrap_err();
            assert!(err.to_string().contains("pin_config_defaults"), "{err}");
        }
    }

    #[test]
    fn gpio_timing_policies_have_backward_compatible_serialized_defaults() {
        let desc = DeviceDescriptor::from_yaml(FIXTURE).unwrap();
        assert_eq!(
            desc.behavior.timer_clock,
            labwired_config::GpioTimerClock::Microseconds
        );
        assert!(!desc.behavior.input_timer_start_on_service);
        assert!(desc.behavior.pin_config_defaults.is_empty());
        let yaml = serde_yaml::to_string(&desc).unwrap();
        let restored = DeviceDescriptor::from_yaml(&yaml).unwrap();
        assert_eq!(restored.behavior.timer_clock, desc.behavior.timer_clock);
        let rotary = DeviceDescriptor::embedded("rotary_encoder")
            .unwrap()
            .unwrap();
        let yaml = serde_yaml::to_string(&rotary).unwrap();
        let restored = DeviceDescriptor::from_yaml(&yaml).unwrap();
        assert_eq!(
            restored.behavior.timer_clock,
            labwired_config::GpioTimerClock::CyclesFloor
        );
        assert!(restored.behavior.input_timer_start_on_service);
        assert_eq!(restored.behavior.pin_config_defaults["clk_pin"], "PA0");
        for invalid in [
            "  timer_clock: cpu_ticks\n",
            "  input_timer_start_on_service: later\n",
        ] {
            let yaml = FIXTURE.replace("  outputs: [OUT]", &format!("{invalid}  outputs: [OUT]"));
            assert!(DeviceDescriptor::from_yaml(&yaml).is_err());
        }
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
