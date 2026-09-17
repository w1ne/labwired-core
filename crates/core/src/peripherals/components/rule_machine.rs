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
    /// The word a register would put on the wire RIGHT NOW: its stored value
    /// for an ordinary register, and the fully encoded measurement — through
    /// `scale_from`, `clamp_from`, `calendar:` and the rest — for one with a
    /// `source:`. See [`labwired_config::expr::Expr::Reported`] for why this
    /// is neither [`reg`](Self::reg) nor [`input`](Self::input).
    ///
    /// `None` ⇒ no such register, or a part with no register file at all.
    fn reported(&self, name: &str) -> Option<i64>;
    /// Store a register's word, bypassing `write_mask` — this is the DEVICE
    /// writing its own register, not the master writing it.
    fn set_reg(&mut self, name: &str, value: u32);
    /// `(shift, mask)` of a named `bits:` field. `None` ⇒ no such field.
    fn field_bits(&self, register: &str, field: &str) -> Option<(u8, u32)>;
    /// A SimInput channel's value as an integer: passed through the `encode:`
    /// of a register that sources the key when one exists, else truncated.
    fn input(&self, key: &str) -> i64;
    /// Assign a SimInput channel, in the SAME integer domain
    /// [`input`](Self::input) reads back — the exact inverse, so a rule that
    /// writes what it read changes nothing.
    ///
    /// Required rather than defaulted. A default no-op would compile, pass
    /// every unit test, and wire nothing: a `set_input:` in a descriptor would
    /// parse, validate, run, and silently do nothing on whichever transport
    /// forgot to implement it.
    fn set_input(&mut self, key: &str, value: i64);
}

/// A [`RuleCtx`] for a part with no register map. Every register lookup misses,
/// which is the truth for a pins-only part.
pub struct PinOnlyCtx<'a> {
    /// Input channel values in engineering units.
    ///
    /// `&mut` because [`RuleCtx::set_input`] writes here: a pins-only or
    /// stream part's channels ARE its whole measurable state, so a rule that
    /// advances a free-running quantity has nowhere else to put it.
    pub slots: &'a mut BTreeMap<String, f64>,
    /// Per-channel `expr_scale` (see
    /// [`labwired_config::InputSpec::expr_scale`]): the factor that turns an
    /// engineering value into the integer count the part's own protocol shifts.
    /// A channel absent from the map scales by 1.0, which is what every
    /// descriptor written before the key existed means.
    pub expr_scale: &'a BTreeMap<String, f64>,
}

impl RuleCtx for PinOnlyCtx<'_> {
    fn reg(&self, _name: &str) -> Option<u32> {
        None
    }
    fn reported(&self, _name: &str) -> Option<i64> {
        None
    }
    fn set_reg(&mut self, _name: &str, _value: u32) {}
    fn field_bits(&self, _register: &str, _field: &str) -> Option<(u8, u32)> {
        None
    }
    fn input(&self, key: &str) -> i64 {
        let value = self.slots.get(key).copied().unwrap_or(0.0);
        let scale = self.expr_scale.get(key).copied().unwrap_or(1.0);
        // Rounded, not truncated: this is a unit conversion, and truncating one
        // biases every reading toward zero by up to a whole count.
        (value * scale).round() as i64
    }

    fn set_input(&mut self, key: &str, value: i64) {
        // The exact inverse of `input` above: divide back out of the
        // `expr_scale` domain. A channel the part does not declare is DROPPED
        // rather than created — validation refuses such a name at load, so
        // anything arriving here is declared, and inventing a slot would make
        // a typo look like it worked.
        if !self.slots.contains_key(key) {
            return;
        }
        let scale = self.expr_scale.get(key).copied().unwrap_or(1.0);
        let engineering = if scale == 0.0 {
            0.0
        } else {
            value as f64 / scale
        };
        self.slots.insert(key.to_string(), engineering);
    }
}

/// A FIFO's `fill:` block with every expression parsed.
#[derive(Debug)]
struct CompiledFill {
    timer: String,
    when: Option<Expr>,
    /// `(expression, width in bits)`, MSB-first in declaration order.
    pack: Vec<(Expr, u8)>,
}

impl CompiledFill {
    fn compile(spec: &FifoSpec) -> Result<Option<Self>> {
        let Some(fill) = &spec.fill else {
            return Ok(None);
        };
        let parse = |src: &str, what: &str| -> Result<Expr> {
            Expr::parse(src)
                .map_err(|e| anyhow::anyhow!("fifo '{}' {what}: {e} — in `{src}`", spec.name))
        };
        let total: u32 = fill.pack.iter().map(|f| u32::from(f.width_bits)).sum();
        anyhow::ensure!(
            total <= 63,
            "fifo '{}' packs {total} bits into one entry; the limit is 63 (an entry is one i64, \
             which holds two 3-axis 16-bit samples or six 10-bit ones)",
            spec.name
        );
        anyhow::ensure!(
            !fill.pack.is_empty(),
            "fifo '{}' declares a `fill:` with an empty `pack:`, so every entry would be zero",
            spec.name
        );
        Ok(Some(Self {
            timer: fill.timer.clone(),
            when: match &fill.when {
                Some(src) => Some(parse(src, "fill.when")?),
                None => None,
            },
            pack: fill
                .pack
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    Ok((
                        parse(&f.expr, &format!("fill.pack[{i}].expr"))?,
                        f.width_bits,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        }))
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
    /// Per-FIFO compiled `fill:` — the guard and the packed component
    /// expressions, parsed ONCE at load. `None` for a FIFO with no `fill:`,
    /// which is one filled only by explicit `push:` actions.
    fifo_fills: Vec<Option<CompiledFill>>,
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
            fifo_fills: behavior
                .fifos
                .iter()
                .map(CompiledFill::compile)
                .collect::<Result<Vec<_>>>()?,
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

    // ── FIFO streams ───────────────────────────────────────────────────────

    /// Fill every FIFO whose `fill.timer` is `timer` and whose guard passes.
    ///
    /// Called by the owning device on each firing of that timer, BEFORE the
    /// `timer:` rules run — so a rule guarded on `fifo_len(samples)` sees the
    /// sample this tick produced, which is what a watermark rule needs.
    ///
    /// Returns whether anything was pushed, so the caller can skip the
    /// register reflection when nothing moved.
    pub fn fill_on_timer(&mut self, timer: &str, ctx: &mut dyn RuleCtx) -> bool {
        let mut pushed = false;
        for i in 0..self.fifo_fills.len() {
            let Some(fill) = &self.fifo_fills[i] else {
                continue;
            };
            if fill.timer != timer {
                continue;
            }
            // Taken out so the guard and the components can be evaluated while
            // `self` is borrowed for `Env`.
            let fill = self.fifo_fills[i].take().expect("checked above");
            let passes = match &fill.when {
                Some(guard) => self.eval(guard, &*ctx) != 0,
                None => true,
            };
            if passes {
                let mut entry: i64 = 0;
                for (expr, width) in &fill.pack {
                    let value = self.eval(expr, &*ctx);
                    let bits = u32::from(*width).min(63);
                    let mask: i64 = if bits >= 63 {
                        i64::MAX
                    } else {
                        (1i64 << bits) - 1
                    };
                    // MSB-first in declaration order: the first component ends
                    // up in the high bits, which is the order a burst read
                    // walks them out in.
                    entry = (entry << bits) | (value & mask);
                }
                self.push_entry(i, entry);
                pushed = true;
            }
            self.fifo_fills[i] = Some(fill);
        }
        pushed
    }

    /// One packed component of a FIFO's OLDEST entry, sign-extended out of its
    /// declared width. `None` when the FIFO is empty or has no such slot —
    /// which is what makes a register with `fifo:` fall through to its live
    /// `source:` in bypass mode.
    pub fn fifo_peek(&self, name: &str, slot: u8) -> Option<i64> {
        let i = self.fifo_index(name)?;
        let fill = self.fifo_fills[i].as_ref()?;
        let entry = *self.fifos[i].front()?;
        let slot = usize::from(slot);
        if slot >= fill.pack.len() {
            return None;
        }
        // Components were packed MSB-first, so slot `s` sits above every
        // component after it.
        let below: u32 = fill.pack[slot + 1..]
            .iter()
            .map(|(_, w)| u32::from(*w))
            .sum();
        let bits = u32::from(fill.pack[slot].1).min(63);
        let mask: i64 = if bits >= 63 {
            i64::MAX
        } else {
            (1i64 << bits) - 1
        };
        let raw = (entry >> below) & mask;
        // Sign-extend: an accelerometer's axis is two's complement, and a
        // register serving it must report -1 rather than 0xFFFF.
        if bits < 63 && raw & (1 << (bits - 1)) != 0 {
            Some(raw | !mask)
        } else {
            Some(raw)
        }
    }

    /// Discard a FIFO's oldest entry. Returns whether one was there.
    pub fn fifo_pop(&mut self, name: &str) -> bool {
        match self.fifo_index(name) {
            Some(i) => self.fifos[i].pop_front().is_some(),
            None => false,
        }
    }

    /// Reflect every FIFO's depth into the registers it declares: the `count:`
    /// field and the `watermark:` bit.
    ///
    /// Called after every fill and every drain. The watermark FOLLOWS the depth
    /// unless the part declared `latch: true`, which is what makes a driver's
    /// "drain until the watermark drops" loop terminate — a latched bit that
    /// only firmware can clear would spin it forever on a part whose datasheet
    /// says otherwise.
    pub fn refresh_fifo_registers(&mut self, ctx: &mut dyn RuleCtx) {
        for i in 0..self.fifo_specs.len() {
            let held = self.fifos[i].len();
            let spec = self.fifo_specs[i].clone();
            if let Some(count) = &spec.count {
                Self::write_field(ctx, &count.register, &count.field, held as u32);
            }
            let Some(wm) = &spec.watermark else { continue };
            let threshold = match (wm.entries, &wm.entries_from) {
                (Some(n), _) => n,
                (None, Some(f)) => match ctx.field_bits(&f.register, &f.field) {
                    Some((shift, mask)) => {
                        ((ctx.reg(&f.register).unwrap_or(0) & mask) >> shift) as usize
                    }
                    None => continue,
                },
                // Validation refuses a watermark with neither.
                (None, None) => continue,
            };
            // A threshold of zero means "any entry at all"; a FIFO holding
            // nothing never raises the bit, which is what empty means.
            let reached = held > 0 && held >= threshold.max(1);
            if !reached && wm.latch {
                continue;
            }
            Self::write_field(ctx, &wm.set.register, &wm.set.field, u32::from(reached));
        }
    }

    /// Store `value` into a named bit-field of a register, leaving the rest.
    fn write_field(ctx: &mut dyn RuleCtx, register: &str, field: &str, value: u32) {
        let Some((shift, mask)) = ctx.field_bits(register, field) else {
            return;
        };
        let prev = ctx.reg(register).unwrap_or(0);
        let next = (prev & !mask) | ((value << shift) & mask);
        if next != prev {
            ctx.set_reg(register, next);
        }
    }

    /// Push one already-packed entry, honouring the declared depth and
    /// overflow policy. Shared by [`fill_on_timer`](Self::fill_on_timer) and
    /// the `push:` action so a streamed entry and a rule-pushed one overflow
    /// the same way.
    fn push_entry(&mut self, i: usize, value: i64) {
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
        self.fifos[i].push_back(value);
    }

    /// Run a list of actions that is NOT attached to an event.
    ///
    /// The `uart_device` primitive's command table is the caller: a matched
    /// [`UartResponse`](labwired_config::UartResponse) may carry `do:` actions,
    /// and those are the same [`CompiledAction`]s a rule runs, applied through
    /// the same `apply`. Giving the table its own interpreter is exactly how a
    /// `set:` inside a response would come to mean something different from a
    /// `set:` inside a rule.
    ///
    /// The recursion guard is honoured, so an action that somehow re-entered
    /// the machine is dropped rather than looping.
    pub fn run_actions(&mut self, actions: &[CompiledAction], ctx: &mut dyn RuleCtx) {
        if self.firing {
            return;
        }
        self.firing = true;
        for action in actions {
            self.apply(action, ctx);
        }
        self.firing = false;
    }

    /// Evaluate one compiled expression against this machine and `ctx`.
    ///
    /// Public because a `uart:` block's guards and templates live OUTSIDE the
    /// rule list but must see exactly the same `var()` / `state` / `input()`
    /// the rules see. Two environments is how an `unsolicited:` guard and the
    /// rule that feeds it would come to disagree.
    pub fn eval_expr(&self, e: &Expr, ctx: &dyn RuleCtx) -> i64 {
        self.eval(e, ctx)
    }

    /// Render a [`Template`](labwired_config::Template) against this machine
    /// and `ctx`. Same environment as [`eval_expr`](Self::eval_expr).
    pub fn render_template(
        &self,
        template: &labwired_config::Template,
        ctx: &dyn RuleCtx,
    ) -> String {
        template.render(&Env { m: self, ctx })
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
                self.push_entry(i, v);
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
            CompiledAction::SetInput { key, value } => {
                // ⚠️ No `Event::Input` is raised. A rule that fed its own
                // trigger would be a loop, and `fire`'s recursion guard would
                // drop the re-entry silently rather than run it — so the rule
                // is that a rule-driven assignment is not an outside event,
                // stated here and in `Action::SetInput`.
                let v = self.eval(value, &*ctx);
                ctx.set_input(key, v);
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
    fn reported(&self, name: &str) -> i64 {
        self.ctx.reported(name).unwrap_or(0)
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
        fn reported(&self, name: &str) -> Option<i64> {
            self.values.get(name).map(|v| i64::from(*v))
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
        fn set_input(&mut self, key: &str, value: i64) {
            self.inputs.insert(key.to_string(), value);
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
