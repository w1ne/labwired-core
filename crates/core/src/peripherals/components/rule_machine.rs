// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The Tier-2 rule machine** — the one engine behind `behavior.rules:`.
//!
//! What it is
//! ==========
//! A part's internal life, held in four things a register map cannot express:
//! a **state**, **variables**, **FIFOs** and **timers** — plus a queue of pin
//! levels the part wants to drive. Events come in
//! ([`RuleMachine::fire`]); the declared rules run in order; registers are
//! changed through the [`RuleCtx`] the owning device supplies, and the pin
//! queue is drained by the bus.
//!
//! One machine, three owners:
//! [`GenericI2cDevice`](super::declarative_i2c::GenericI2cDevice),
//! [`GenericSpiDevice`](super::declarative_spi::GenericSpiDevice), and the
//! `gpio_device` primitive
//! ([`DeclarativeGpioDevice`](super::declarative_gpio::DeclarativeGpioDevice))
//! for parts that are only pins. None of the three knows anything about the
//! others' transport.
//!
//! Ordering, stated once
//! =====================
//! Rules fire in DECLARATION ORDER for a given event, and each rule's `do:` list
//! runs to completion before the next rule is considered. A rule may `goto:`, and
//! later rules **in the same event** then evaluate against the NEW state. The
//! `when:` guard of a later rule therefore sees every register write, variable
//! assignment and state change an earlier rule made. That is the documented
//! contract: it makes a two-step sequence writable in one event, at the price of
//! rule order mattering — which is why the ordering is stated here, in the
//! schema docs (`labwired_config::rules`), and in every ported descriptor.
//!
//! Where events come from
//! ======================
//! `write` / `read` are raised by the owning device AFTER the built-in register
//! semantics have run, so a rule always sees the post-side-effect register: a
//! `write:` rule reads the STORED word (write-mask applied, self-clearing bits
//! already gone) and `written` holds the raw value the master put on the wire.
//! `start` / `stop` / `cs_select` / `cs_release` come from the transaction
//! boundaries; `timer:` from the central device-time drive
//! ([`Peripheral::advance_attached_device_time_us`](crate::Peripheral::advance_attached_device_time_us));
//! `pin:` from the bus service pass; `frame` from the framing spec.
//!
//! Totality
//! ========
//! Expressions are parsed once at load ([`labwired_config::compile_rules`]) and
//! evaluate totally (see `labwired_config::expr`): a division by zero is 0 and a
//! fidelity note, never a panic. A rule can therefore not take the machine down,
//! which matters because rules arrive from a part document a generator wrote.

use std::collections::{BTreeMap, VecDeque};

use anyhow::Result;
use labwired_config::expr::{EvalCtx, Expr};
use labwired_config::{
    compile_rules, CompiledAction, CompiledRule, DeviceBehavior, Event, FifoOverflow, FifoSpec,
    PinEdge, RegBits,
};

/// What a rule may read and change on the device that owns the machine.
///
/// Deliberately four methods over primitives. A device that has no registers
/// (the `gpio_device` primitive) implements the register half as "nothing",
/// which is exactly right rather than a stub: a part with no register map has
/// no registers for a rule to touch, and a descriptor naming one fails
/// validation at load.
pub trait RuleCtx {
    /// A register's current stored word. `None` ⇒ no such register.
    fn reg(&self, name: &str) -> Option<u32>;
    /// Store a register's word, bypassing `write_mask` — this is the DEVICE
    /// writing its own register, not the master writing it.
    fn set_reg(&mut self, name: &str, value: u32);
    /// `(shift, mask)` of a named `bits:` field. `None` ⇒ no such field.
    fn field_bits(&self, register: &str, field: &str) -> Option<(u8, u32)>;
    /// A SimInput channel's value as an integer: passed through the `encode:`
    /// of a register that sources the key when one exists, else truncated.
    fn input(&self, key: &str) -> i64;
}

/// A [`RuleCtx`] for a part with no register map. Every register lookup misses,
/// which is the truth for a pins-only part.
pub struct PinOnlyCtx<'a> {
    /// Input channel values in engineering units.
    pub slots: &'a BTreeMap<String, f64>,
}

impl RuleCtx for PinOnlyCtx<'_> {
    fn reg(&self, _name: &str) -> Option<u32> {
        None
    }
    fn set_reg(&mut self, _name: &str, _value: u32) {}
    fn field_bits(&self, _register: &str, _field: &str) -> Option<(u8, u32)> {
        None
    }
    fn input(&self, key: &str) -> i64 {
        self.slots.get(key).copied().unwrap_or(0.0) as i64
    }
}

/// The machine.
#[derive(Debug)]
pub struct RuleMachine {
    rules: Vec<CompiledRule>,
    states: Vec<String>,
    state: String,
    var_resets: BTreeMap<String, i64>,
    vars: BTreeMap<String, i64>,
    fifo_specs: Vec<FifoSpec>,
    fifos: Vec<VecDeque<i64>>,
    /// Timer NAMES the part declares, so a `timer:` action can say which one it
    /// means. The timers themselves live in the device's
    /// [`TimerBank`](super::declarative_regs::TimerBank) — see the module note.
    timer_names: Vec<String>,
    /// Timers a rule asked to start or stop, for the device to hand to its
    /// bank. Drained by [`take_timer_requests`](Self::take_timer_requests).
    pending_timers: Vec<(String, bool)>,
    outputs: Vec<String>,
    /// Last level driven on each output, so the queue carries TRANSITIONS only.
    pin_levels: BTreeMap<String, bool>,
    /// `(role, level)` waiting for the bus to put on a pad.
    pending_pins: Vec<(String, bool)>,
    /// Device time in µs, advanced by the central drive.
    elapsed_us: u64,
    /// The value the master just wrote, for `written` inside a `write:` rule.
    written: i64,
    /// Fidelity counters — see [`Self::fidelity_notes`].
    divide_by_zero: std::cell::Cell<u64>,
    fifo_overflows: u64,
    /// Recursion guard: an action must never re-enter `fire`.
    firing: bool,
}

impl RuleMachine {
    /// Build from a `behavior:` block. `Ok(None)` when the part declares no
    /// Tier-2 machinery at all, so a Tier-1 device allocates and checks nothing.
    pub fn from_behavior(behavior: &DeviceBehavior) -> Result<Option<Self>> {
        if behavior.rules.is_empty()
            && behavior.timers.is_empty()
            && behavior.fifos.is_empty()
            && behavior.outputs.is_empty()
            && behavior.states.is_empty()
            && behavior.vars.is_empty()
        {
            return Ok(None);
        }
        let rules = compile_rules(&behavior.rules).map_err(|e| anyhow::anyhow!("{e}"))?;
        let state = behavior.states.first().cloned().unwrap_or_default();
        Ok(Some(Self {
            rules,
            states: behavior.states.clone(),
            state,
            var_resets: behavior.vars.clone(),
            vars: behavior.vars.clone(),
            fifos: behavior.fifos.iter().map(|_| VecDeque::new()).collect(),
            fifo_specs: behavior.fifos.clone(),
            timer_names: behavior.timers.iter().map(|t| t.name.clone()).collect(),
            pending_timers: Vec::new(),
            outputs: behavior.outputs.clone(),
            pin_levels: BTreeMap::new(),
            pending_pins: Vec::new(),
            elapsed_us: 0,
            written: 0,
            divide_by_zero: std::cell::Cell::new(0),
            fifo_overflows: 0,
            firing: false,
        }))
    }

    /// The part's current state name.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Every declared state, in declaration order (first = reset).
    pub fn states(&self) -> &[String] {
        &self.states
    }

    /// A variable's current value (diagnostics and tests).
    pub fn var(&self, name: &str) -> i64 {
        self.vars.get(name).copied().unwrap_or(0)
    }

    /// A FIFO's current depth (diagnostics and tests).
    pub fn fifo_len(&self, name: &str) -> usize {
        self.fifo_index(name)
            .map(|i| self.fifos[i].len())
            .unwrap_or(0)
    }

    /// The level this machine last drove on `role`. `None` ⇒ never driven.
    pub fn pin_level(&self, role: &str) -> Option<bool> {
        self.pin_levels.get(role).copied()
    }

    /// Pin roles this part declares as outputs.
    pub fn outputs(&self) -> &[String] {
        &self.outputs
    }

    /// Whether any rule listens for an event of this shape at all, so a hot
    /// path can skip building one.
    pub fn listens_for_writes(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r.on, Event::Write { .. }))
    }

    /// Same, for reads.
    pub fn listens_for_reads(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r.on, Event::Read { .. }))
    }

    /// Whether this part drives any pin, so the bus knows to service it.
    pub fn drives_pins(&self) -> bool {
        !self.outputs.is_empty()
    }

    /// Take the queued `(pin role, level)` transitions for the bus to apply.
    pub fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.pending_pins)
    }

    /// Fidelity notes worth recording in the census: the approximations this
    /// machine made rather than trapped on.
    pub fn fidelity_notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.divide_by_zero.get() > 0 {
            out.push(format!(
                "rule expression divided by zero {} time(s); the result was 0 rather than a trap",
                self.divide_by_zero.get()
            ));
        }
        if self.fifo_overflows > 0 {
            out.push(format!(
                "{} FIFO entr(ies) were dropped on overflow",
                self.fifo_overflows
            ));
        }
        out
    }

    fn fifo_index(&self, name: &str) -> Option<usize> {
        self.fifo_specs.iter().position(|f| f.name == name)
    }

    /// Timer requests a rule queued, for the device to apply to its bank.
    /// `(name, start)` — `false` stops it.
    pub fn take_timer_requests(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.pending_timers)
    }

    // ── events ─────────────────────────────────────────────────────────────

    /// Raise an event. `written` is the value the master just put on the wire
    /// (0 for every event that is not a write).
    pub fn fire(&mut self, event: &Event, written: i64, ctx: &mut dyn RuleCtx) {
        if self.rules.is_empty() || self.firing {
            return;
        }
        self.firing = true;
        self.written = written;
        // The rule list is immutable for the life of the machine; taking it out
        // is what lets an action hold `&mut self` while the loop reads a rule.
        let rules = std::mem::take(&mut self.rules);
        for rule in &rules {
            if !self.event_matches(&rule.on, event, written, &*ctx) {
                continue;
            }
            // A guard is true when it evaluates non-zero. It is evaluated
            // against the state as it stands NOW, so an earlier rule's `goto:`
            // or register write is already visible here — the ordering contract
            // in the module note.
            if let Some(guard) = &rule.when {
                if self.eval(guard, &*ctx) == 0 {
                    continue;
                }
            }
            for action in &rule.actions {
                self.apply(action, ctx);
            }
        }
        self.rules = rules;
        self.written = 0;
        self.firing = false;
    }

    /// Record the device's elapsed µs. The machine does NOT schedule anything:
    /// its owner's [`TimerBank`](super::declarative_regs::TimerBank) decides
    /// when a timer is due and calls [`fire`](Self::fire) with
    /// `Event::Timer`. This only keeps `elapsed_us` for diagnostics.
    pub fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(us);
    }

    /// Device time in µs as this machine has observed it.
    pub fn elapsed_us(&self) -> u64 {
        self.elapsed_us
    }

    fn event_matches(
        &self,
        rule_on: &Event,
        actual: &Event,
        written: i64,
        ctx: &dyn RuleCtx,
    ) -> bool {
        match (rule_on, actual) {
            (
                Event::Write {
                    register: want,
                    field,
                },
                Event::Write {
                    register: got,
                    field: None,
                },
            ) => {
                if want != got {
                    return false;
                }
                // `write: REG.FIELD` fires when the master's write left any bit
                // of that field SET. That is what "wrote the field" means for
                // every command bit a datasheet describes; a rule that wants the
                // zero case writes `on: { write: REG }` with a `when:` guard.
                match field {
                    None => true,
                    Some(f) => ctx
                        .field_bits(want, f)
                        .is_some_and(|(_, mask)| (written as u32) & mask != 0),
                }
            }
            (Event::Read { register: a }, Event::Read { register: b }) => a == b,
            (Event::Timer { name: a }, Event::Timer { name: b }) => a == b,
            (Event::Input { key: a }, Event::Input { key: b }) => a == b,
            (
                Event::Pin {
                    name: a,
                    edge: want,
                },
                Event::Pin {
                    name: b,
                    edge: happened,
                },
            ) => a == b && (*want == PinEdge::Any || want == happened),
            (a, b) => a == b,
        }
    }

    fn eval(&self, e: &Expr, ctx: &dyn RuleCtx) -> i64 {
        let env = Env { m: self, ctx };
        e.eval(&env)
    }

    fn apply(&mut self, action: &CompiledAction, ctx: &mut dyn RuleCtx) {
        match action {
            CompiledAction::Set(bits) => self.mask_write(bits, true, ctx),
            CompiledAction::Clear(bits) => self.mask_write(bits, false, ctx),
            CompiledAction::Write { register, value } => {
                let v = self.eval(value, &*ctx);
                ctx.set_reg(register, v as u32);
            }
            CompiledAction::Goto { state } => {
                // A `goto:` to an undeclared state is refused at LOAD
                // (`validate_rule_names`), so anything arriving here is declared.
                self.state = state.clone();
            }
            CompiledAction::Timer { name, start } => {
                // The machine owns no timer state: it queues the request and
                // the device applies it to the ONE bank. A name the part does
                // not declare is refused at load, so anything arriving here is
                // real.
                if self.timer_names.iter().any(|t| t == name) {
                    self.pending_timers.push((name.clone(), *start));
                }
            }
            CompiledAction::Push { fifo, value } => {
                let Some(i) = self.fifo_index(fifo) else {
                    return;
                };
                let v = match value {
                    Some(e) => self.eval(e, &*ctx),
                    None => match &self.fifo_specs[i].source {
                        Some(key) => ctx.input(key),
                        // A push with neither a `value:` nor a `source:` is
                        // refused at load; 0 here would be a silent sample.
                        None => return,
                    },
                };
                let depth = self.fifo_specs[i].depth;
                if self.fifos[i].len() >= depth {
                    self.fifo_overflows = self.fifo_overflows.saturating_add(1);
                    match self.fifo_specs[i].overflow {
                        FifoOverflow::DropOldest => {
                            self.fifos[i].pop_front();
                        }
                        // The part stopped sampling: the incoming entry is lost.
                        FifoOverflow::DropNewest => return,
                    }
                }
                self.fifos[i].push_back(v);
            }
            CompiledAction::Pop { fifo } => {
                if let Some(i) = self.fifo_index(fifo) {
                    self.fifos[i].pop_front();
                }
            }
            CompiledAction::Pin { name, level } => {
                // The level is an EXPRESSION, so one rule can mirror a register
                // bit onto a pad (`level: "field(PORT.P3)"`). Non-zero is high.
                let high = self.eval(level, &*ctx) != 0;
                self.drive_pin(name, high);
            }
            CompiledAction::Var { name, value } => {
                let v = self.eval(value, &*ctx);
                self.vars.insert(name.clone(), v);
            }
        }
    }

    /// Queue a pin transition. Transition-only: re-driving the level a pin
    /// already holds costs nothing and produces no bus write, which is the same
    /// contract [`DevicePins`](crate::bus::DevicePins) keeps at the pad.
    pub fn drive_pin(&mut self, role: &str, level: bool) {
        if self.pin_levels.get(role) == Some(&level) {
            return;
        }
        self.pin_levels.insert(role.to_string(), level);
        self.pending_pins.push((role.to_string(), level));
    }

    fn mask_write(&mut self, bits: &RegBits, set: bool, ctx: &mut dyn RuleCtx) {
        let mask = match (&bits.field, bits.mask) {
            (Some(f), _) => match ctx.field_bits(&bits.register, f) {
                Some((_, m)) => m,
                None => return,
            },
            (None, Some(m)) => m,
            (None, None) => return,
        };
        let prev = ctx.reg(&bits.register).unwrap_or(0);
        let next = if set { prev | mask } else { prev & !mask };
        if next != prev {
            ctx.set_reg(&bits.register, next);
        }
    }

    /// Reset every piece of machine state to its power-on value. Used by a
    /// device that models a soft reset; the pin queue is cleared, not drained,
    /// because a reset drops whatever it had not yet driven.
    pub fn reset(&mut self) {
        self.state = self.states.first().cloned().unwrap_or_default();
        self.vars = self.var_resets.clone();
        for f in self.fifos.iter_mut() {
            f.clear();
        }
        self.pending_timers.clear();
        self.pending_pins.clear();
        self.pin_levels.clear();
    }
}

/// The evaluation environment: the machine's own state plus the owning device's
/// registers. Built per expression and dropped immediately, which is what lets
/// `apply` hold `&mut self` right after.
struct Env<'a> {
    m: &'a RuleMachine,
    ctx: &'a dyn RuleCtx,
}

impl EvalCtx for Env<'_> {
    fn reg(&self, name: &str) -> i64 {
        i64::from(self.ctx.reg(name).unwrap_or(0))
    }
    fn field(&self, register: &str, field: &str) -> i64 {
        match self.ctx.field_bits(register, field) {
            Some((shift, mask)) => {
                let word = self.ctx.reg(register).unwrap_or(0);
                i64::from((word & mask) >> shift)
            }
            None => 0,
        }
    }
    fn var(&self, name: &str) -> i64 {
        self.m.vars.get(name).copied().unwrap_or(0)
    }
    fn input(&self, key: &str) -> i64 {
        self.ctx.input(key)
    }
    fn fifo_len(&self, name: &str) -> i64 {
        self.m.fifo_len(name) as i64
    }
    fn written(&self) -> i64 {
        self.m.written
    }
    fn state(&self) -> &str {
        &self.m.state
    }
    fn note_divide_by_zero(&self) {
        self.m.divide_by_zero.set(self.m.divide_by_zero.get() + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::DeviceDescriptor;

    /// A register file behind the [`RuleCtx`], for testing the machine on its
    /// own — the engine half, without a bus.
    #[derive(Default)]
    struct Regs {
        values: BTreeMap<String, u32>,
        fields: BTreeMap<(String, String), (u8, u32)>,
        inputs: BTreeMap<String, i64>,
    }

    impl RuleCtx for Regs {
        fn reg(&self, name: &str) -> Option<u32> {
            self.values.get(name).copied()
        }
        fn set_reg(&mut self, name: &str, value: u32) {
            self.values.insert(name.to_string(), value);
        }
        fn field_bits(&self, register: &str, field: &str) -> Option<(u8, u32)> {
            self.fields
                .get(&(register.to_string(), field.to_string()))
                .copied()
        }
        fn input(&self, key: &str) -> i64 {
            self.inputs.get(key).copied().unwrap_or(0)
        }
    }

    fn machine(behavior_yaml: &str) -> RuleMachine {
        let desc = DeviceDescriptor::from_yaml(&format!(
            "type: test_rule_machine\nbehavior:\n  primitive: i2c_device\n{behavior_yaml}"
        ))
        .expect("descriptor parses");
        RuleMachine::from_behavior(&desc.behavior)
            .expect("rules compile")
            .expect("the descriptor declares Tier-2 machinery")
    }

    #[test]
    fn a_tier1_descriptor_builds_no_machine() {
        let desc = DeviceDescriptor::from_yaml(
            "type: t\nbehavior:\n  primitive: i2c_device\n  i2c: { default_address: 0x10 }\n",
        )
        .unwrap();
        assert!(RuleMachine::from_behavior(&desc.behavior)
            .unwrap()
            .is_none());
    }

    /// A `timer:` action does not schedule anything here — it queues a request
    /// the owning device hands to the ONE
    /// [`TimerBank`](super::super::declarative_regs::TimerBank). The machine
    /// keeping its own deadlines is exactly how a rule and an `on_fire` would
    /// come to disagree about when the part ticked.
    #[test]
    fn a_timer_action_queues_a_request_rather_than_scheduling() {
        let mut m = machine(
            r#"  timers:
    - { name: sample, period_us: 1000, start: manual, on_fire: [] }
  rules:
    - on: start
      do: [ { timer: sample, start: true } ]
    - on: stop
      do: [ { timer: sample, start: false } ]
"#,
        );
        let mut regs = Regs::default();
        m.fire(&Event::Start, 0, &mut regs);
        assert_eq!(m.take_timer_requests(), vec![("sample".to_string(), true)]);
        assert!(m.take_timer_requests().is_empty(), "the queue drains");
        m.fire(&Event::Stop, 0, &mut regs);
        assert_eq!(m.take_timer_requests(), vec![("sample".to_string(), false)]);
    }

    /// A timer EVENT still reaches the rules — it just arrives from the device,
    /// which is what makes there be one clock.
    #[test]
    fn a_timer_event_from_the_device_runs_its_rules() {
        let mut m = machine(
            r#"  outputs: [INT]
  vars: { n: 0 }
  timers:
    - { name: sample, period_us: 1000, on_fire: [] }
  rules:
    - on: { timer: sample }
      do: [ { var: n, value: "var(n) + 1" }, { pin: INT, level: 1 } ]
"#,
        );
        let mut regs = Regs::default();
        m.fire(
            &Event::Timer {
                name: "sample".into(),
            },
            0,
            &mut regs,
        );
        assert_eq!(m.var("n"), 1);
        assert_eq!(m.take_pin_drives(), vec![("INT".to_string(), true)]);
        // A second firing re-runs the rule but the LEVEL did not change, so the
        // queue stays empty: it carries transitions.
        m.fire(
            &Event::Timer {
                name: "sample".into(),
            },
            0,
            &mut regs,
        );
        assert_eq!(m.var("n"), 2);
        assert!(m.take_pin_drives().is_empty());
    }

    #[test]
    fn rules_fire_in_order_and_a_goto_is_visible_to_later_rules() {
        let mut m = machine(
            r#"  states: [idle, armed]
  vars: { hit: 0 }
  rules:
    - on: start
      do: [ { goto: armed } ]
    - on: start
      when: "state == armed"
      do: [ { var: hit, value: 1 } ]
"#,
        );
        let mut regs = Regs::default();
        assert_eq!(
            m.state(),
            "idle",
            "the first declared state is the reset one"
        );
        m.fire(&Event::Start, 0, &mut regs);
        assert_eq!(m.state(), "armed");
        assert_eq!(
            m.var("hit"),
            1,
            "a later rule in the SAME event sees the new state"
        );
    }

    #[test]
    fn a_write_field_event_fires_only_when_the_bit_was_written() {
        let mut m = machine(
            r#"  vars: { n: 0 }
  rules:
    - on: { write: CMD.GO }
      do: [ { var: n, value: "var(n) + 1" } ]
"#,
        );
        let mut regs = Regs::default();
        regs.fields.insert(("CMD".into(), "GO".into()), (3, 0b1000));
        let ev = Event::Write {
            register: "CMD".into(),
            field: None,
        };
        m.fire(&ev, 0x00, &mut regs);
        assert_eq!(m.var("n"), 0, "the field bit was clear");
        m.fire(&ev, 0x08, &mut regs);
        assert_eq!(m.var("n"), 1, "the field bit was set");
    }

    #[test]
    fn set_and_clear_go_through_named_bits() {
        let mut m = machine(
            r#"  rules:
    - on: start
      do: [ { set: STATUS.RDY } ]
    - on: stop
      do: [ { clear: STATUS.RDY } ]
    - on: frame
      do: [ { set: { register: STATUS, mask: 0x30 } } ]
"#,
        );
        let mut regs = Regs::default();
        regs.fields
            .insert(("STATUS".into(), "RDY".into()), (0, 0b1));
        regs.set_reg("STATUS", 0x80);
        m.fire(&Event::Start, 0, &mut regs);
        assert_eq!(regs.reg("STATUS"), Some(0x81));
        m.fire(&Event::Frame, 0, &mut regs);
        assert_eq!(regs.reg("STATUS"), Some(0xB1));
        m.fire(&Event::Stop, 0, &mut regs);
        assert_eq!(regs.reg("STATUS"), Some(0xB0));
    }

    #[test]
    fn a_fifo_overflows_by_its_declared_policy() {
        let mut m = machine(
            r#"  fifos:
    - { name: old, depth: 2, overflow: drop_oldest }
    - { name: new, depth: 2, overflow: drop_newest }
  rules:
    - on: frame
      do: [ { push: old, value: "written" }, { push: new, value: "written" } ]
"#,
        );
        let mut regs = Regs::default();
        for v in 1..=4 {
            m.fire(&Event::Frame, v, &mut regs);
        }
        assert_eq!(m.fifo_len("old"), 2);
        assert_eq!(m.fifo_len("new"), 2);
        assert!(
            m.fidelity_notes()
                .iter()
                .any(|n| n.contains("dropped on overflow")),
            "{:?}",
            m.fidelity_notes()
        );
    }

    #[test]
    fn a_push_with_no_value_takes_the_fifos_source_channel() {
        let mut m = machine(
            r#"  fifos:
    - { name: samples, depth: 4, source: weight }
  rules:
    - on: frame
      do: [ { push: samples } ]
"#,
        );
        let mut regs = Regs::default();
        regs.inputs.insert("weight".into(), 4242);
        m.fire(&Event::Frame, 0, &mut regs);
        assert_eq!(m.fifo_len("samples"), 1);
        assert_eq!(
            Expr::parse("fifo_len(samples)")
                .unwrap()
                .eval(&Env { m: &m, ctx: &regs }),
            1
        );
    }

    #[test]
    fn a_pin_edge_rule_matches_only_its_declared_edge() {
        let mut m = machine(
            r#"  vars: { rises: 0, falls: 0 }
  outputs: [OUT]
  pins: { SCK: sck_pin }
  rules:
    - on: { pin: SCK, edge: rising }
      do: [ { var: rises, value: "var(rises) + 1" } ]
    - on: { pin: SCK, edge: falling }
      do: [ { var: falls, value: "var(falls) + 1" } ]
"#,
        );
        let mut regs = Regs::default();
        m.fire(
            &Event::Pin {
                name: "SCK".into(),
                edge: PinEdge::Rising,
            },
            0,
            &mut regs,
        );
        m.fire(
            &Event::Pin {
                name: "SCK".into(),
                edge: PinEdge::Falling,
            },
            0,
            &mut regs,
        );
        m.fire(
            &Event::Pin {
                name: "SCK".into(),
                edge: PinEdge::Rising,
            },
            0,
            &mut regs,
        );
        assert_eq!((m.var("rises"), m.var("falls")), (2, 1));
    }

    #[test]
    fn a_divide_by_zero_in_a_guard_is_a_note_not_a_panic() {
        let mut m = machine(
            r#"  vars: { n: 0, d: 0 }
  rules:
    - on: frame
      when: "100 / var(d) > 0"
      do: [ { var: n, value: 1 } ]
"#,
        );
        let mut regs = Regs::default();
        m.fire(&Event::Frame, 0, &mut regs);
        assert_eq!(m.var("n"), 0);
        assert!(
            m.fidelity_notes()
                .iter()
                .any(|n| n.contains("divided by zero")),
            "{:?}",
            m.fidelity_notes()
        );
    }

    #[test]
    fn reset_restores_the_reset_state_and_variables() {
        let mut m = machine(
            r#"  states: [idle, busy]
  vars: { n: 7 }
  rules:
    - on: start
      do: [ { goto: busy }, { var: n, value: 99 } ]
"#,
        );
        let mut regs = Regs::default();
        m.fire(&Event::Start, 0, &mut regs);
        assert_eq!((m.state(), m.var("n")), ("busy", 99));
        m.reset();
        assert_eq!((m.state(), m.var("n")), ("idle", 7));
    }
}
