// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The **`uart_device` primitive** — a part whose whole interface is a byte
//! stream.
//!
//! What it covers
//! ==============
//! Two shapes that a register map cannot reach because the part has no
//! registers:
//!
//! * a **command shell** — firmware writes a line, the part answers a line
//!   (HC-05, SIM800L, and every AT modem);
//! * an **unsolicited stream** — the part talks on its own clock whether or not
//!   anyone asked (a GPS emitting NMEA once a second).
//!
//! Both were hand-written Rust, and the three models this primitive replaces
//! were the same file three times: the same line buffer, the same 128-byte cap,
//! the same `byte as char` cast, the same `poll` that pops one byte. Only the
//! `handle_line` ladder differed — which is the part of a datasheet that is a
//! TABLE, and a table is data.
//!
//! One machine, four owners
//! ========================
//! This is the fourth owner of [`RuleMachine`]: `GenericI2cDevice`,
//! `GenericSpiDevice`, `DeclarativeGpioDevice` and now this. A `uart_device` is
//! not a second rule engine — it is a third transport for the one that exists,
//! and every `states:` / `vars:` / `timers:` / `rules:` key means here exactly
//! what it means on the other three.
//!
//! Its clock
//! =========
//! [`UartStreamDevice::poll`] is handed `elapsed_us` by the hosting UART, which
//! credits the tick interval on the FIRST poll of a tick and zero to the rest.
//! That is the device's time, and it drives the SAME [`TimerBank`] every other
//! declarative part uses — so an `unsolicited:` sentence and a `timer:` rule
//! cannot disagree about when the part ticked.
//!
//! ⚠️ **The timers do not drift.** The hand-written NEO-6M reset its
//! accumulator to zero after each sentence and only accumulated while its
//! output queue was EMPTY, so its period was really "500 ms plus however long
//! the last sentence took to clock out" — about 570 ms at one byte per
//! millisecond. [`TimerBank`] reschedules from the DEADLINE, so the port emits
//! on a true 500 ms grid. This is a deliberate difference and
//! `uart_migration_parity.rs` asserts both halves of it.
//!
//! What it deliberately is NOT
//! ===========================
//! A protocol stack. Nothing here knows what Bluetooth pairing, GSM
//! registration or a GPS almanac is, and a descriptor that answers `+CSQ: 20,0`
//! is declaring a CONSTANT, visibly, in the part document — which is what the
//! models it replaces were doing invisibly in Rust.

use std::collections::{BTreeMap, VecDeque};

use anyhow::{anyhow, Context, Result};
use labwired_config::{
    ByteTemplate, Checksum, DeviceDescriptor, Event, Template, TemplateWrap, UartMatch,
    UartResponse, UartSpec, UartUnsolicited,
};

use super::declarative_regs::{apply_timing_action, TimerBank};
use super::rule_machine::{PinOnlyCtx, RuleMachine};
use crate::peripherals::device::UartStreamDevice;
use crate::peripherals::noise::ChannelNoise;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// Where one register-table word comes from.
enum RegSrc {
    /// An expression: `input(KEY)` (so the channel's `expr_scale` applies), a
    /// computed value, or `var(NAME)` for a var-backed register.
    Expr(labwired_config::expr::Expr),
    Const(i64),
}

/// A compiled `write_regs:` entry: first register, word count, body offset.
struct WriteRegs {
    first: labwired_config::expr::Expr,
    count: labwired_config::expr::Expr,
    data_at: usize,
}

/// The descriptor with every response-pattern capture declared as a rule
/// variable, so `var(addr)` names resolve exactly like any other variable. A
/// borrow when the part has no captures, which is every part written before
/// binary frames.
pub(crate) fn with_capture_vars(
    desc: &DeviceDescriptor,
) -> Result<std::borrow::Cow<'_, DeviceDescriptor>> {
    let Some(spec) = desc.behavior.uart.as_ref() else {
        return Ok(std::borrow::Cow::Borrowed(desc));
    };
    let captures = spec.capture_names();
    if captures.is_empty() {
        return Ok(std::borrow::Cow::Borrowed(desc));
    }
    let mut out = desc.clone();
    for name in captures {
        anyhow::ensure!(
            !desc.behavior.vars.contains_key(&name),
            "uart_device '{}': the byte-pattern capture `{name}` has the same name as a declared \
             var; rename one of them",
            desc.r#type
        );
        out.behavior.vars.insert(name, 0);
    }
    Ok(std::borrow::Cow::Owned(out))
}

/// A stream-only declarative device.
pub struct DeclarativeUartDevice {
    id: String,
    /// The part's frame shape, command table and unsolicited output.
    spec: UartSpec,
    /// Compiled `do:` actions of each [`UartResponse`], indexed alongside
    /// `spec.responses`. Compiled ONCE at construction: the engine never parses
    /// an expression on the wire path.
    response_actions: Vec<Vec<labwired_config::CompiledAction>>,
    /// Compiled `when:` guard of each [`UartUnsolicited`] entry.
    unsolicited_guards: Vec<Option<labwired_config::expr::Expr>>,
    machine: RuleMachine,
    /// The part's own timers — the SAME bank every other declarative primitive
    /// uses. A `uart_device` has no register file, so a timer's `on_fire:`
    /// register actions have nowhere to land and are dropped; what an
    /// `unsolicited:` entry and a rule listen for is the timer EVENT.
    timers: TimerBank,
    /// Measurement slots in engineering units, keyed by input-channel key.
    slots: BTreeMap<String, f64>,
    /// Per-channel `expr_scale` — the counts per engineering unit a template's
    /// `input()` sees. See [`labwired_config::InputSpec::expr_scale`].
    expr_scale: BTreeMap<String, f64>,
    channels: std::borrow::Cow<'static, [InputChannel]>,
    /// Per-channel seeded Gaussian noise, keyed by channel key. Present only
    /// for a channel whose descriptor (or a `config:` override) gives it a
    /// sigma. Sampled ONCE per rendered template, never per byte — a
    /// multi-byte sentence must not re-roll halfway through, or a driver that
    /// parses it sees a position that is in two places at once.
    noise: BTreeMap<String, ChannelNoise>,
    /// Bytes waiting to go onto the RX line, one per [`poll`](Self::poll).
    out_queue: VecDeque<u8>,
    /// Answers that have not come due yet: `(deadline_us, bytes)`, kept in
    /// insertion order. A modem's `delay_us` is what makes a driver's timeout
    /// worth testing at all.
    pending: VecDeque<(u64, Vec<u8>)>,
    /// Bytes of the frame currently arriving on TX.
    rx_frame: Vec<u8>,
    /// Device time in µs, credited by the hosting UART.
    elapsed_us: u64,
    /// `frames.framing: silence`: frames are raw bytes that end on line idle.
    raw_frames: bool,
    /// Idle time that ends a silence frame, µs.
    gap_us: u64,
    /// Whole polls' worth of idle counted since the last byte arrived. Only
    /// polls that saw NO byte count, so a frame never ends early: the figure
    /// is a lower bound on the real idle.
    idle_us: u64,
    /// A byte arrived since the last time device time was credited.
    rx_heard: bool,
    /// Checksum carried at the end of every silence frame.
    frame_check: Option<Checksum>,
    /// Frames dropped for a bad checksum.
    check_errors: u64,
    /// Paced by its own baud (`pace: device` or silence framing).
    paced: bool,
    /// Nanoseconds one character occupies the line. Only meaningful when paced.
    char_ns: u64,
    /// Nanoseconds of line time earned and not yet spent on a byte.
    tx_credit_ns: u64,
    /// Pattern-capture names, bound as vars before a frame's rules run.
    capture_names: Vec<String>,
    /// Compiled `when:` guard of each response.
    response_when: Vec<Option<labwired_config::expr::Expr>>,
    /// The register table behind `regs(first, count)`.
    reg_srcs: BTreeMap<i64, RegSrc>,
    /// Registers a request may write, and the var each one stores into.
    reg_vars: BTreeMap<i64, String>,
    /// Compiled `write_regs:` of each response, indexed alongside
    /// `spec.responses`.
    response_writes: Vec<Option<WriteRegs>>,
}

impl std::fmt::Debug for DeclarativeUartDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarativeUartDevice")
            .field("id", &self.id)
            .field("state", &self.machine.state())
            .field("queued", &self.out_queue.len())
            .finish()
    }
}

impl DeclarativeUartDevice {
    /// Build from a validated descriptor.
    pub fn new(
        id: String,
        descriptor: &DeviceDescriptor,
        channels: std::borrow::Cow<'static, [InputChannel]>,
    ) -> Result<Self> {
        let spec =
            descriptor.behavior.uart.clone().ok_or_else(|| {
                anyhow!("uart_device '{}' has no `uart:` block", descriptor.r#type)
            })?;
        // A stream part with no state, no timers and no rules is legal — an AT
        // shell that only answers from a constant table is exactly that — so
        // unlike `gpio_device` an absent machine is not an error. It is
        // replaced by an empty one so every code path below has a machine to
        // hold the vars a template might read.
        let with_caps = with_capture_vars(descriptor)?;
        let machine = match RuleMachine::from_behavior(&with_caps.behavior)? {
            Some(m) => m,
            None => RuleMachine::from_behavior(&labwired_config::DeviceBehavior {
                // One declared var, so `from_behavior` returns a machine rather
                // than `None`. Nothing reads it; it exists so the stream path
                // is not written twice, once with a machine and once without.
                vars: [("_".to_string(), 0i64)].into_iter().collect(),
                ..descriptor.behavior.clone()
            })?
            .expect("a behavior with a var always yields a machine"),
        };
        let response_actions = spec
            .responses
            .iter()
            .map(|r| {
                labwired_config::compile_rules(&[labwired_config::Rule {
                    min_hold_us: None,
                    on: Event::Frame,
                    when: None,
                    actions: r.actions.clone(),
                }])
                .map(|mut rules| std::mem::take(&mut rules[0].actions))
                .map_err(|e| anyhow!("{e}"))
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| {
                format!(
                    "uart_device '{}' has a response whose `do:` will not compile",
                    descriptor.r#type
                )
            })?;
        let unsolicited_guards = spec
            .unsolicited
            .iter()
            .map(|u| match &u.when {
                None => Ok(None),
                Some(src) => labwired_config::expr::Expr::parse(src)
                    .map(Some)
                    .map_err(|e| anyhow!("uart.unsolicited `when: {src}`: {e}")),
            })
            .collect::<Result<Vec<_>>>()?;

        let response_when = spec
            .responses
            .iter()
            .map(|r| match &r.when {
                None => Ok(None),
                Some(src) => labwired_config::expr::Expr::parse(src)
                    .map(Some)
                    .map_err(|e| anyhow!("uart.responses `when: {src}`: {e}")),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut reg_srcs = BTreeMap::new();
        let mut reg_vars = BTreeMap::new();
        for r in &spec.regs {
            let parse = |text: &str, what: &str| {
                labwired_config::expr::Expr::parse(text)
                    .map_err(|e| anyhow!("uart.regs {what} `{text}`: {e}"))
            };
            let src = if let Some(key) = &r.input {
                RegSrc::Expr(parse(&format!("input({key})"), "input")?)
            } else if let Some(text) = &r.expr {
                RegSrc::Expr(parse(text, "expr")?)
            } else if let Some(name) = &r.var {
                reg_vars.insert(i64::from(r.reg), name.clone());
                RegSrc::Expr(parse(&format!("var({name})"), "var")?)
            } else {
                RegSrc::Const(r.value.unwrap_or(0))
            };
            reg_srcs.insert(i64::from(r.reg), src);
        }
        let response_writes = spec
            .responses
            .iter()
            .map(|r| {
                r.write_regs
                    .as_ref()
                    .map(|w| {
                        Ok::<_, anyhow::Error>(WriteRegs {
                            first: labwired_config::expr::Expr::parse(&w.first)
                                .map_err(|e| anyhow!("write_regs.first `{}`: {e}", w.first))?,
                            count: labwired_config::expr::Expr::parse(&w.count)
                                .map_err(|e| anyhow!("write_regs.count `{}`: {e}", w.count))?,
                            data_at: usize::from(w.data_at),
                        })
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        let paced = spec.paced_by_device();
        let char_ns = spec.char_time_ns().unwrap_or(0);
        let gap_us = spec.gap_us().unwrap_or(0);
        let raw_frames = spec.frames.is_silence();
        let frame_check = spec.frames.check;
        let capture_names = spec.capture_names();

        let mut slots = BTreeMap::new();
        let mut expr_scale = BTreeMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
                if let Some(scale) = input.expr_scale {
                    expr_scale.insert(input.key.clone(), scale);
                }
            }
        }
        let mut noise = BTreeMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                let sigma = input.noise_sigma.unwrap_or(0.0);
                let bias = input.bias.unwrap_or(0.0);
                if sigma > 0.0 || bias != 0.0 {
                    noise.insert(
                        input.key.clone(),
                        ChannelNoise::new(0, &id, &input.key, sigma, bias, input.thermal_tau_s),
                    );
                }
            }
        }
        Ok(Self {
            noise,
            id,
            timers: TimerBank::new(&descriptor.behavior.timers),
            spec,
            response_actions,
            unsolicited_guards,
            machine,
            slots,
            expr_scale,
            channels,
            out_queue: VecDeque::new(),
            pending: VecDeque::new(),
            rx_frame: Vec::new(),
            elapsed_us: 0,
            raw_frames,
            gap_us,
            idle_us: 0,
            rx_heard: false,
            frame_check,
            check_errors: 0,
            paced,
            char_ns,
            // The line has been idle since power-up, so the first byte may go
            // out at once.
            tx_credit_ns: char_ns,
            capture_names,
            response_when,
            reg_srcs,
            reg_vars,
            response_writes,
        })
    }

    /// Override the part's line rate from a placement's `baud:` config. The
    /// character time and the silence gap follow it, so a Modbus slave placed at
    /// 19200 baud ends its frames after 3.5 characters at 19200, not at the
    /// descriptor's default. Only a part that opted in to its own baud has one.
    pub fn set_baud(&mut self, baud: u32) {
        if baud == 0 || !self.paced {
            return;
        }
        self.spec.baud = Some(baud);
        self.char_ns = self.spec.char_time_ns().unwrap_or(0);
        self.gap_us = self.spec.gap_us().unwrap_or(0);
        self.tx_credit_ns = self.char_ns;
    }

    /// Seed a declared var from a placement's `config:` (a Modbus slave's
    /// `address`). Returns false when the part declares no such var.
    pub fn seed_var(&mut self, name: &str, value: i64) -> bool {
        if self.machine.has_var(name) {
            self.machine.set_var(name, value);
            true
        } else {
            false
        }
    }

    /// Seed a measurement slot from a `config:` override, like every other
    /// declarative primitive does.
    pub fn seed_input(&mut self, key: &str, value: f64) {
        if self.channels.iter().any(|c| c.key == key) {
            self.slots.insert(key.to_string(), value);
        }
    }

    /// Override one channel's noise sigma from a `config:` value, the same
    /// `noise_sigma_key` knob the declarative I²C kit honours. Sigma 0 removes
    /// the noise state entirely, so a placement can turn it off and get the
    /// byte-identical quiet stream back.
    pub fn set_channel_noise_sigma(&mut self, key: &str, sigma: f64) {
        if !self.channels.iter().any(|c| c.key == key) {
            return;
        }
        match self.noise.get_mut(key) {
            Some(existing) if sigma > 0.0 => {
                *existing =
                    ChannelNoise::new(0, &self.id, key, sigma, existing.bias(), existing.tau_s());
            }
            Some(existing) => {
                let bias = existing.bias();
                if bias == 0.0 {
                    self.noise.remove(key);
                } else {
                    *existing = ChannelNoise::new(0, &self.id, key, 0.0, bias, existing.tau_s());
                }
            }
            None if sigma > 0.0 => {
                self.noise.insert(
                    key.to_string(),
                    ChannelNoise::new(0, &self.id, key, sigma, 0.0, None),
                );
            }
            None => {}
        }
    }

    /// The slot map a template renders against: the truth values, with each
    /// noisy channel sampled ONCE. Returns a borrow of the real map when no
    /// channel is noisy, which is every part written before `noise_sigma`.
    fn observed_slots(&mut self) -> BTreeMap<String, f64> {
        let mut out = self.slots.clone();
        let now = self.elapsed_us;
        for (key, noise) in self.noise.iter_mut() {
            if let Some(v) = out.get_mut(key) {
                *v = noise.sample(*v, Some(now));
            }
        }
        out
    }

    /// A measurement slot's current value in engineering units — the seam the
    /// wasm inspect bridge reads a GPS fix through, so it no longer downcasts
    /// to a concrete model per part.
    pub fn slot(&self, key: &str) -> Option<f64> {
        self.slots.get(key).copied()
    }

    /// Read-only view of the rule machine, for tests and diagnostics.
    pub fn rule_machine(&self) -> &RuleMachine {
        &self.machine
    }

    fn fire(&mut self, event: Event) {
        let mut ctx = PinOnlyCtx {
            expr_precision: &BTreeMap::new(),
            slots: &mut self.slots,
            expr_scale: &self.expr_scale,
        };
        self.machine.fire(&event, 0, &mut ctx);
        self.drain_timer_requests();
    }

    fn drain_timer_requests(&mut self) {
        for (name, start) in self.machine.take_timer_requests() {
            if start {
                self.timers.start_named(&name, self.elapsed_us);
            } else {
                self.timers.stop_named(&name);
            }
        }
    }

    /// Queue a rendered answer, honouring `delay_us`.
    fn emit(&mut self, text: &str, wrap: TemplateWrap, delay_us: Option<u64>) {
        self.emit_bytes(wrap.apply(text).into_bytes(), delay_us);
    }

    /// Queue already-rendered bytes, honouring `delay_us`.
    fn emit_bytes(&mut self, bytes: Vec<u8>, delay_us: Option<u64>) {
        if bytes.is_empty() {
            return;
        }
        match delay_us.filter(|d| *d > 0) {
            None => self.out_queue.extend(bytes),
            Some(d) => self
                .pending
                .push_back((self.elapsed_us.saturating_add(d), bytes)),
        }
    }

    /// Render a template. Noisy channels are sampled here, once per template,
    /// which is exactly where the hand-written NEO-6M sampled them.
    fn render(&mut self, t: &Template) -> String {
        if self.noise.is_empty() {
            // Disjoint field borrows: the machine is read while the slots are
            // held mutably, which is what lets one `RuleCtx` type serve both
            // the read-only render and the `set_input:` write path.
            let ctx = PinOnlyCtx {
                expr_precision: &BTreeMap::new(),
                slots: &mut self.slots,
                expr_scale: &self.expr_scale,
            };
            return self.machine.render_template(t, &ctx);
        }
        let mut slots = self.observed_slots();
        let ctx = PinOnlyCtx {
            expr_precision: &BTreeMap::new(),
            slots: &mut slots,
            expr_scale: &self.expr_scale,
        };
        self.machine.render_template(t, &ctx)
    }

    /// Move every answer whose delay has expired into the output queue, in the
    /// order the answers were produced. Order is preserved even when a later
    /// answer has a shorter delay: a modem does not reorder its own replies.
    fn release_due(&mut self) {
        while let Some((deadline, _)) = self.pending.front() {
            if *deadline > self.elapsed_us {
                break;
            }
            let (_, bytes) = self.pending.pop_front().expect("front was Some");
            self.out_queue.extend(bytes);
        }
    }

    /// Credit `us` of device time: fire due timers, and for each firing render
    /// the `unsolicited:` entries bound to it.
    ///
    /// ⚠️ **Rendering comes BEFORE the timer's rules.** Both halves see the
    /// same pre-firing state, which is what makes an alternating stream
    /// writable: two entries guarded `var(n) % 2 == 0` and `== 1` are mutually
    /// exclusive only because neither sees the increment the rule is about to
    /// make. Running the rules first would make the second guard true in the
    /// same tick and emit both sentences every time.
    fn advance(&mut self, us: u64) {
        if us == 0 {
            return;
        }
        self.elapsed_us = self.elapsed_us.saturating_add(us);
        self.machine.advance_time_us(us);
        if self.raw_frames {
            self.check_silence(us);
        }
        if !self.timers.is_empty() {
            let mut scratch = std::collections::HashMap::new();
            for (name, actions) in self.timers.due_by_timer(self.elapsed_us) {
                // A stream part has no register file; the `on_fire:` actions go
                // to a scratch map so the ONE bank keeps one code path.
                for action in &actions {
                    apply_timing_action(action, &mut scratch);
                }
                self.emit_unsolicited(&name);
                self.fire(Event::Timer { name });
            }
        }
        self.drain_timer_requests();
        self.release_due();
    }

    fn emit_unsolicited(&mut self, timer: &str) {
        for i in 0..self.spec.unsolicited.len() {
            let entry: &UartUnsolicited = &self.spec.unsolicited[i];
            if entry.timer != timer {
                continue;
            }
            let wrap = entry.wrap;
            if let Some(guard) = &self.unsolicited_guards[i] {
                let ctx = PinOnlyCtx {
                    expr_precision: &BTreeMap::new(),
                    slots: &mut self.slots,
                    expr_scale: &self.expr_scale,
                };
                if self.machine.eval_expr(guard, &ctx) == 0 {
                    continue;
                }
            }
            let template = self.spec.unsolicited[i].template.clone();
            let text = self.render(&template);
            self.emit(&text, wrap, None);
        }
    }

    /// A frame arrived on the TX line: answer it from the command table.
    ///
    /// ⚠️ **A frame that is empty after trimming produces nothing.** Every AT
    /// shell in the wild does this, and the models replaced here did it
    /// explicitly (`if t.is_empty() { return; }`). Without it a host that sends
    /// `AT\r\n` — the overwhelmingly common spelling — would get two answers,
    /// because the `\r` completes the frame and the `\n` completes an empty one.
    fn on_frame(&mut self, frame: &[u8]) {
        let text: String = frame.iter().map(|&b| b as char).collect();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let ignore_case = self.spec.frames.ignore_case;
        let probe = if ignore_case {
            trimmed.to_ascii_uppercase()
        } else {
            trimmed.to_string()
        };
        self.fire(Event::Frame);
        let Some(i) = self
            .spec
            .responses
            .iter()
            .position(|r| r.r#match.matches(&probe, ignore_case))
        else {
            // No entry matched and no catch-all was declared: the part is
            // silent, which is what a real shell does with a command it does
            // not implement and a driver's timeout is the observable.
            return;
        };
        let actions = std::mem::take(&mut self.response_actions[i]);
        if !actions.is_empty() {
            let mut ctx = PinOnlyCtx {
                expr_precision: &BTreeMap::new(),
                slots: &mut self.slots,
                expr_scale: &self.expr_scale,
            };
            self.machine.run_actions(&actions, &mut ctx);
        }
        self.response_actions[i] = actions;
        self.drain_timer_requests();
        let entry: &UartResponse = &self.spec.responses[i];
        let (respond, wrap, delay) = (entry.respond.clone(), entry.wrap, entry.delay_us);
        if let Some(t) = respond {
            let text = self.render(&t);
            self.emit(&text, wrap, delay);
        }
    }
}

impl DeclarativeUartDevice {
    /// Evaluate against the machine and the (possibly noisy) slots, the same
    /// environment a text template renders in.
    fn eval_with<R>(&mut self, f: impl FnOnce(&RuleMachine, &PinOnlyCtx<'_>) -> R) -> R {
        if self.noise.is_empty() {
            let ctx = PinOnlyCtx {
                expr_precision: &BTreeMap::new(),
                slots: &mut self.slots,
                expr_scale: &self.expr_scale,
            };
            return f(&self.machine, &ctx);
        }
        let mut slots = self.observed_slots();
        let ctx = PinOnlyCtx {
            expr_precision: &BTreeMap::new(),
            slots: &mut slots,
            expr_scale: &self.expr_scale,
        };
        f(&self.machine, &ctx)
    }

    /// Render a `respond_bytes:` list. Captures are already bound as vars.
    fn render_bytes(&mut self, t: &ByteTemplate) -> Vec<u8> {
        let regs = std::mem::take(&mut self.reg_srcs);
        let out = self.eval_with(|machine, ctx| {
            let mut eval = |e: &labwired_config::expr::Expr| machine.eval_expr(e, ctx);
            let mut reg = |n: i64| match regs.get(&n) {
                Some(RegSrc::Expr(e)) => machine.eval_expr(e, ctx),
                Some(RegSrc::Const(v)) => *v,
                None => 0,
            };
            t.render(&mut eval, &mut reg)
        });
        self.reg_srcs = regs;
        out
    }

    /// Bind a frame's captures as rule variables, zeroing the rest.
    fn bind_captures(&mut self, caps: &[(String, i64)]) {
        for name in &self.capture_names {
            self.machine.set_var(name, 0);
        }
        for (name, v) in caps {
            self.machine.set_var(name, *v);
        }
    }

    /// End a silence frame once the line has been idle long enough.
    ///
    /// `us` is the real device time just credited. A poll in which a byte
    /// arrived restarts the count and contributes nothing, so the count is a
    /// lower bound on the true idle and a frame can end late by at most one
    /// poll, never early.
    fn check_silence(&mut self, us: u64) {
        if self.rx_frame.is_empty() || self.rx_heard {
            self.rx_heard = false;
            self.idle_us = 0;
            return;
        }
        self.idle_us = self.idle_us.saturating_add(us);
        if self.idle_us >= self.gap_us {
            self.idle_us = 0;
            let frame = std::mem::take(&mut self.rx_frame);
            self.on_raw_frame(&frame);
        }
    }

    /// A binary frame closed by silence: verify its checksum, match it against
    /// the table, bind captures, and answer.
    ///
    /// Captures are bound from the FIRST entry whose pattern matches before the
    /// frame's rules fire, so a `rules:` entry on `frame` sees them. Entries
    /// are then tried in order and the first whose pattern matches and whose
    /// `when:` holds answers; a later entry's captures are bound just before
    /// its own guard is read.
    fn on_raw_frame(&mut self, frame: &[u8]) {
        if frame.is_empty() {
            return;
        }
        let mut body = frame;
        if let Some(ck) = self.frame_check {
            let w = ck.width();
            if frame.len() <= w {
                self.check_errors += 1;
                return;
            }
            let (head, tail) = frame.split_at(frame.len() - w);
            if ck.wire_bytes(head) != tail {
                // What a Modbus slave does: no answer to a corrupt frame.
                self.check_errors += 1;
                return;
            }
            body = head;
        }
        let text: String = body.iter().map(|&b| b as char).collect();
        let mut hits: Vec<(usize, Vec<(String, i64)>)> = Vec::new();
        for (i, r) in self.spec.responses.iter().enumerate() {
            match &r.r#match {
                UartMatch::Bytes(p) => {
                    if let Some(caps) = p.matches(body) {
                        hits.push((
                            i,
                            caps.into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
                        ));
                    }
                }
                other => {
                    if other.matches(&text, false) {
                        hits.push((i, Vec::new()));
                    }
                }
            }
        }
        match hits.first() {
            Some((_, caps)) => {
                let caps = caps.clone();
                self.bind_captures(&caps);
            }
            None => self.bind_captures(&[]),
        }
        self.fire(Event::Frame);
        let mut chosen = None;
        for (i, caps) in hits {
            self.bind_captures(&caps);
            if let Some(guard) = self.response_when[i].clone() {
                let pass = self.eval_with(|machine, ctx| machine.eval_expr(&guard, ctx)) != 0;
                if !pass {
                    continue;
                }
            }
            chosen = Some(i);
            break;
        }
        let Some(i) = chosen else {
            return;
        };
        let actions = std::mem::take(&mut self.response_actions[i]);
        if !actions.is_empty() {
            let mut ctx = PinOnlyCtx {
                expr_precision: &BTreeMap::new(),
                slots: &mut self.slots,
                expr_scale: &self.expr_scale,
            };
            self.machine.run_actions(&actions, &mut ctx);
        }
        self.response_actions[i] = actions;
        self.drain_timer_requests();
        self.apply_write_regs(i, body);
        let entry: &UartResponse = &self.spec.responses[i];
        let (respond, bytes, wrap, delay) = (
            entry.respond.clone(),
            entry.respond_bytes.clone(),
            entry.wrap,
            entry.delay_us,
        );
        if let Some(t) = bytes {
            let out = self.render_bytes(&t);
            self.emit_bytes(out, delay);
        } else if let Some(t) = respond {
            let text = self.render(&t);
            self.emit(&text, wrap, delay);
        }
    }

    /// Store the words a `write_regs:` entry names. Words are read big-endian
    /// from `body` and stored, truncated to 16 bits, into the var behind each
    /// register. A register with no var behind it is skipped.
    fn apply_write_regs(&mut self, response: usize, body: &[u8]) {
        let Some(w) = &self.response_writes[response] else {
            return;
        };
        let (data_at, first_e, count_e) = (w.data_at, w.first.clone(), w.count.clone());
        let (first, count) =
            self.eval_with(|m, c| (m.eval_expr(&first_e, c), m.eval_expr(&count_e, c)));
        let count = count.clamp(0, labwired_config::uart_binary::MAX_REG_WORDS);
        for k in 0..count {
            let at = data_at + 2 * k as usize;
            let Some(word) = body.get(at..at + 2) else {
                break;
            };
            let value = i64::from(u16::from_be_bytes([word[0], word[1]]));
            if let Some(var) = self.reg_vars.get(&first.wrapping_add(k)).cloned() {
                self.machine.set_var(&var, value);
            }
        }
    }

    /// Frames dropped so far for a bad checksum.
    pub fn check_errors(&self) -> u64 {
        self.check_errors
    }

    /// Release one byte if the line has had time to carry it. The credit never
    /// banks more than one character while nothing is queued, so a response
    /// that follows a long idle starts at once but then runs at the part's
    /// baud, not in a burst.
    fn paced_pop(&mut self, elapsed_us: u32) -> Option<u8> {
        if self.out_queue.is_empty() && self.pending.is_empty() {
            self.tx_credit_ns = self.tx_credit_ns.min(self.char_ns);
        }
        self.tx_credit_ns = self
            .tx_credit_ns
            .saturating_add(u64::from(elapsed_us) * 1000);
        if self.out_queue.is_empty() {
            self.tx_credit_ns = self.tx_credit_ns.min(self.char_ns);
            return None;
        }
        if self.tx_credit_ns < self.char_ns {
            return None;
        }
        self.tx_credit_ns -= self.char_ns;
        self.out_queue.pop_front()
    }
}

impl UartStreamDevice for DeclarativeUartDevice {
    fn paced_by_device(&self) -> bool {
        self.paced
    }

    /// A silence-framed part speaks raw binary frames, so its link must stay off
    /// the console capture sink: request bytes such as `01 03 … C4 0B` are not
    /// console text and would splice into whatever the firmware prints.
    fn carries_protocol_octets(&self) -> bool {
        self.raw_frames
    }

    fn device_id(&self) -> Option<&str> {
        Some(&self.id)
    }

    fn declared_baud(&self) -> Option<u32> {
        if self.paced {
            self.spec.baud
        } else {
            None
        }
    }

    fn max_bytes_per_tick(&self) -> usize {
        if self.paced && self.char_ns > 0 {
            // One bus tick is nominally 1 ms: as many characters as fit in it.
            ((1_000_000 / self.char_ns) as usize + 1).min(256)
        } else {
            1
        }
    }

    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
        // Time is credited FIRST and unconditionally — including while the
        // queue is draining. The hand-written NEO-6M did the opposite (it only
        // accumulated when its queue was empty), which is what made its "1 Hz"
        // stream really run at one sentence per 500 ms PLUS the drain time.
        self.advance(u64::from(elapsed_us));
        if self.paced {
            return self.paced_pop(elapsed_us);
        }
        self.out_queue.pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if self.raw_frames {
            self.rx_heard = true;
            if self.rx_frame.len() < usize::from(self.spec.frames.max_bytes) {
                self.rx_frame.push(byte);
            }
            return;
        }
        let frames = &self.spec.frames;
        if let Some(length) = frames.length.filter(|n| *n > 0) {
            self.rx_frame.push(byte);
            if self.rx_frame.len() >= usize::from(length) {
                let frame = std::mem::take(&mut self.rx_frame);
                self.on_frame(&frame);
            }
            return;
        }
        if frames.terminator.as_bytes().contains(&byte) {
            let frame = std::mem::take(&mut self.rx_frame);
            self.on_frame(&frame);
            return;
        }
        // Past the cap the byte is DROPPED rather than growing the buffer: a
        // firmware bug that never sends a terminator must not become an
        // unbounded allocation inside the simulator. The hand-written shells
        // spelled the same rule as `if self.rx_line.len() < 128`.
        if self.rx_frame.len() < usize::from(frames.max_bytes) {
            self.rx_frame.push(byte);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl SimInput for DeclarativeUartDevice {
    fn input_channels(&self) -> &[InputChannel] {
        &self.channels
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.slots.insert(key.to_string(), value);
        self.fire(Event::Input {
            key: key.to_string(),
        });
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }

    fn set_component_id(&mut self, id: String) {
        self.id = id;
    }
}

// ─── validation ────────────────────────────────────────────────────────────

/// Validate the static descriptor contract for the `uart_device` primitive.
///
/// Kept separate from construction (like every sibling primitive) so manifest
/// preflight can reject an incomplete pack without building anything.
pub(crate) fn validate_descriptor(desc: &DeviceDescriptor) -> Result<()> {
    anyhow::ensure!(
        desc.behavior.primitive == "uart_device",
        "declarative uart kit requires behavior.primitive: uart_device, got '{}'",
        desc.behavior.primitive
    );
    let spec = desc.behavior.uart.as_ref().ok_or_else(|| {
        anyhow!(
            "uart_device '{}' declares no `uart:` block — it would attach to a UART and never \
             send or answer a byte",
            desc.r#type
        )
    })?;
    anyhow::ensure!(
        desc.behavior.i2c.is_none() && desc.behavior.spi.is_none(),
        "uart_device '{}' also declares a register map; a stream part has no registers, and a \
         rule naming one would silently read zero",
        desc.r#type
    );
    let timers: Vec<String> = desc
        .behavior
        .timers
        .iter()
        .map(|t| t.name.clone())
        .collect();
    let inputs: Vec<String> = desc
        .metadata
        .as_ref()
        .map(|m| m.inputs.iter().map(|i| i.key.clone()).collect())
        .unwrap_or_default();
    labwired_config::validate_uart(spec, &timers, &[], &inputs)
        .with_context(|| format!("uart_device '{}' has an invalid `uart:` block", desc.r#type))?;
    // Response `do:` lists are rules in everything but their trigger, so they
    // compile and are name-checked exactly as `rules:` are.
    let action_rules: Vec<labwired_config::Rule> = spec
        .responses
        .iter()
        .map(|r| labwired_config::Rule {
            min_hold_us: None,
            on: Event::Frame,
            when: None,
            actions: r.actions.clone(),
        })
        .collect();
    labwired_config::compile_rules(&action_rules)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| {
            format!(
                "uart_device '{}' has a response whose `do:` will not compile",
                desc.r#type
            )
        })?;
    let with_caps = with_capture_vars(desc)?;
    super::declarative_gpio::validate_rule_names(&with_caps)?;
    let vars: Vec<String> = with_caps.behavior.vars.keys().cloned().collect();
    let fifos: Vec<String> = desc.behavior.fifos.iter().map(|f| f.name.clone()).collect();
    labwired_config::validate_rule_names(
        &action_rules,
        &labwired_config::RuleNames {
            registers: &[],
            fields: &[],
            states: &desc.behavior.states,
            vars: &vars,
            fifos: &fifos,
            timers: &timers,
            data_ready: &[],
            outputs: &desc.behavior.outputs,
            inputs: &inputs,
            pins: &[],
            // A UART part is a byte stream, not a framed message bus.
            frames: None,
        },
    )
    .with_context(|| {
        format!(
            "uart_device '{}' has a response action naming something it does not declare",
            desc.r#type
        )
    })?;
    labwired_config::compile_rules(&desc.behavior.rules)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("uart_device '{}' has an invalid rule", desc.r#type))?;
    for r in &spec.regs {
        if let Some(name) = &r.var {
            anyhow::ensure!(
                desc.behavior.vars.contains_key(name),
                "uart_device '{}': register {} is backed by var `{name}`, which is not declared \
                 in `vars:`",
                desc.r#type,
                r.reg
            );
        }
    }
    anyhow::ensure!(
        !spec.responses.iter().any(|r| r.write_regs.is_some())
            || spec.regs.iter().any(|r| r.var.is_some()),
        "uart_device '{}' has a `write_regs:` entry but no register declared with `var:`, so \
         nothing a request writes could be stored",
        desc.r#type
    );
    Ok(())
}

// ─── the kit wrapper ───────────────────────────────────────────────────────

/// A `uart_device` descriptor as a [`PeripheralKit`], so a ported stream part
/// keeps its entry in the peripheral MANIFEST — its label, its `config:` keys
/// and its stimulus channels.
///
/// Unlike the GPIO kit this one performs the attach itself, because a UART peer
/// is not a bus resident: it is handed to the hosting UART through
/// `AttachCtx::uart()`, which is the same one call every hand-written UART kit
/// made. There is still exactly one attach path per transport.
pub struct DeclarativeUartKit {
    descriptor: DeviceDescriptor,
    channels: std::borrow::Cow<'static, [InputChannel]>,
    metadata: crate::peripherals::kit::KitMetadata,
}

impl std::fmt::Debug for DeclarativeUartKit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarativeUartKit")
            .field("type", &self.descriptor.r#type)
            .finish()
    }
}

impl DeclarativeUartKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;
        let channels = super::declarative_i2c::owned_channels(&descriptor);
        let metadata = super::declarative_i2c::owned_uart_metadata(&descriptor, channels.clone());
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }

    /// Build the device without attaching it — the seam the migration-parity
    /// tests drive, so a golden transcript is captured from the real engine
    /// rather than from a test-only reimplementation of it.
    pub fn device(&self, id: &str) -> Result<DeclarativeUartDevice> {
        DeclarativeUartDevice::new(id.to_string(), &self.descriptor, self.channels.clone())
    }
}

impl crate::peripherals::kit::PeripheralKit for DeclarativeUartKit {
    fn metadata(&self) -> &crate::peripherals::kit::KitMetadata {
        &self.metadata
    }

    fn attach(&self, ctx: &mut crate::peripherals::kit::AttachCtx<'_>) -> Result<()> {
        let id = ctx.device_id().to_string();
        let mut device = DeclarativeUartDevice::new(id, &self.descriptor, self.channels.clone())?;
        for ch in self.channels.iter() {
            if let Some(v) = ctx.config_f64(ch.key.as_ref()) {
                device.seed_input(ch.key.as_ref(), v);
            }
        }
        // A descriptor may alias a channel to a differently-spelled `config:`
        // key (the NEO-6M's `lat_deg` / `lon_deg` for `lat` / `lon`), so the
        // declared aliases are honoured after the plain keys.
        //
        // `noise_sigma_key` is the same knob the declarative I²C kit honours:
        // one `config:` value over a channel SET, so `noise_sigma: 1e-5` on a
        // placement reaches both lat and lon and nothing else.
        if let Some(meta) = &self.descriptor.metadata {
            for input in &meta.inputs {
                if let Some(key) = &input.config_key {
                    if let Some(v) = ctx.config_f64(key) {
                        device.seed_input(&input.key, v);
                    }
                }
                if let Some(key) = &input.noise_sigma_key {
                    if let Some(sigma) = ctx.config_f64(key) {
                        device.set_channel_noise_sigma(&input.key, sigma);
                    }
                }
            }
        }
        // A placement's `baud:` sets the line rate of a part that is paced by
        // its own baud, and a `config:` key named like a declared var seeds that
        // var (a Modbus slave's `address:`).
        if let Some(baud) = ctx.config_i64("baud").and_then(|b| u32::try_from(b).ok()) {
            device.set_baud(baud);
        }
        for name in self.descriptor.behavior.vars.keys() {
            if let Some(v) = ctx.config_i64(name) {
                device.seed_var(name, v);
            }
        }
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(device));
        Ok(())
    }
}

/// Same bridge the I²C and GPIO kits use: the registry is a `const` slice of
/// `&'static dyn PeripheralKit`, and a descriptor is parsed at runtime.
impl crate::peripherals::kit::PeripheralKit for std::sync::LazyLock<DeclarativeUartKit> {
    fn metadata(&self) -> &crate::peripherals::kit::KitMetadata {
        std::sync::LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut crate::peripherals::kit::AttachCtx<'_>) -> Result<()> {
        std::sync::LazyLock::force(self).attach(ctx)
    }
}

/// Build a kit from an embedded descriptor, panicking with the part's name if
/// the shipped YAML is not valid — which is a build-time fact, checked by
/// `peripheral_kit_gate.rs` on every run.
macro_rules! embedded_uart_kit {
    ($name:ident, $type:literal, $doc:literal) => {
        #[doc = $doc]
        pub static $name: std::sync::LazyLock<DeclarativeUartKit> =
            std::sync::LazyLock::new(|| {
                DeclarativeUartKit::from_yaml(
                    labwired_config::embedded_device_yaml($type)
                        .expect(concat!($type, " descriptor is embedded")),
                )
                .expect(concat!(
                    $type,
                    ".yaml is a valid declarative uart descriptor"
                ))
            });
    };
}

embedded_uart_kit!(
    HC05_KIT,
    "hc-05",
    "HC-05 Bluetooth SPP module (declarative `hc-05.yaml`). Migrated from the \
     hand-written `components/hc05.rs`; `tests/uart_migration_parity.rs` pins \
     the wire transcript."
);
embedded_uart_kit!(
    SIM800L_KIT,
    "sim800l",
    "SIMCom SIM800L GSM module (declarative `sim800l.yaml`). Migrated from the \
     hand-written `components/sim800l.rs`; `tests/uart_migration_parity.rs` \
     pins the wire transcript and names every constant."
);
embedded_uart_kit!(
    NEO6M_KIT,
    "neo6m-gps",
    "u-blox NEO-6M GPS receiver (declarative `neo6m-gps.yaml`). Migrated from \
     the hand-written `components/neo6m.rs`; \
     `tests/uart_migration_parity.rs` pins the NMEA bytes."
);

#[cfg(test)]
mod tests {
    #[test]
    fn schedule_actions_are_rejected_in_uart_responses() {
        let desc: labwired_config::DeviceDescriptor = serde_yaml::from_str(
            r#"
type: uart_schedule_invalid
behavior:
  primitive: uart_device
  uart:
    responses:
      - match: any
        respond: ok
        do: [{emit_schedule: missing}]
"#,
        )
        .unwrap();
        assert!(super::validate_descriptor(&desc).is_err());
    }

    use super::*;

    const SHELL: &str = r#"
type: test_uart_shell
behavior:
  primitive: uart_device
  uart:
    frames: { terminator: "\r\n" }
    responses:
      - { match: { prefix: "AT+VERSION" }, respond: "+VERSION:x\r\nOK\r\n" }
      - { match: "AT", respond: "OK\r\n" }
      - { match: { prefix: "AT+" }, respond: "OK\r\n" }
      - { match: any, respond: "ERROR\r\n" }
"#;

    fn shell() -> DeclarativeUartDevice {
        let desc = DeviceDescriptor::from_yaml(SHELL).expect("fixture parses");
        validate_descriptor(&desc).expect("fixture validates");
        DeclarativeUartDevice::new("sh".into(), &desc, std::borrow::Cow::Borrowed(&[]))
            .expect("constructs")
    }

    fn drain(dev: &mut DeclarativeUartDevice) -> String {
        let mut out = String::new();
        while let Some(b) = dev.poll(0) {
            out.push(b as char);
        }
        out
    }

    fn send(dev: &mut DeclarativeUartDevice, line: &str) {
        for b in line.bytes() {
            dev.on_tx_byte(b);
        }
    }

    #[test]
    fn the_command_table_answers_in_order() {
        let mut dev = shell();
        send(&mut dev, "AT\r\n");
        assert_eq!(drain(&mut dev), "OK\r\n");
        send(&mut dev, "AT+VERSION?\r\n");
        assert_eq!(drain(&mut dev), "+VERSION:x\r\nOK\r\n");
        send(&mut dev, "AT+NAME\r\n");
        assert_eq!(drain(&mut dev), "OK\r\n");
        send(&mut dev, "hello\r\n");
        assert_eq!(drain(&mut dev), "ERROR\r\n");
    }

    /// The CRLF trap: `\r` completes the frame and `\n` completes an empty one.
    /// Without the empty-frame rule every `AT\r\n` answers twice.
    #[test]
    fn crlf_answers_exactly_once() {
        let mut dev = shell();
        send(&mut dev, "AT\r\n");
        assert_eq!(drain(&mut dev), "OK\r\n");
        // A CR-only host is the same one frame.
        send(&mut dev, "AT\r");
        assert_eq!(drain(&mut dev), "OK\r\n");
        // …and so is an LF-only one.
        send(&mut dev, "AT\n");
        assert_eq!(drain(&mut dev), "OK\r\n");
    }

    #[test]
    fn matching_folds_case_by_default() {
        let mut dev = shell();
        send(&mut dev, "at+version?\r\n");
        assert_eq!(drain(&mut dev), "+VERSION:x\r\nOK\r\n");
    }

    #[test]
    fn a_line_past_the_cap_is_truncated_not_grown() {
        let mut dev = shell();
        send(&mut dev, &"X".repeat(5000));
        assert_eq!(dev.rx_frame.len(), 128, "the default cap held");
        send(&mut dev, "\r\n");
        assert_eq!(drain(&mut dev), "ERROR\r\n");
    }

    const STREAM: &str = r#"
type: test_uart_stream
behavior:
  primitive: uart_device
  vars: { n: 0 }
  timers:
    - { name: tick, period_us: 500000, start: on_reset }
  uart:
    frames: { terminator: "\r\n" }
    unsolicited:
      - { timer: tick, when: "var(n) % 2 == 0", template: "A,{input(x):05.2}", wrap: nmea }
      - { timer: tick, when: "var(n) % 2 == 1", template: "B", wrap: nmea }
  rules:
    - on: { timer: tick }
      do: [ { var: n, value: "var(n) + 1" } ]
metadata:
  inputs:
    - { key: x, label: X, unit: u, min: 0, max: 1000, default: 12.34, expr_scale: 100 }
"#;

    fn stream() -> DeclarativeUartDevice {
        let desc = DeviceDescriptor::from_yaml(STREAM).expect("fixture parses");
        validate_descriptor(&desc).expect("fixture validates");
        let channels: std::borrow::Cow<'static, [InputChannel]> =
            super::super::declarative_i2c::owned_channels(&desc);
        DeclarativeUartDevice::new("st".into(), &desc, channels).expect("constructs")
    }

    /// Credit `us` of device time and read out everything the part then says.
    fn tick(dev: &mut DeclarativeUartDevice, us: u32) -> String {
        let mut out = String::new();
        if let Some(b) = dev.poll(us) {
            out.push(b as char);
        }
        out.push_str(&drain(dev));
        out
    }

    /// The ordering contract: both guards see the PRE-rule `n`, so exactly one
    /// entry fires per tick and the two alternate.
    #[test]
    fn unsolicited_entries_alternate_rather_than_both_firing() {
        let mut dev = stream();
        assert_eq!(
            tick(&mut dev, 500_000),
            "$A,12.34*47\r\n",
            "the even entry fired alone, fixed-point from expr_scale"
        );
        assert_eq!(
            tick(&mut dev, 500_000),
            "$B*42\r\n",
            "the odd entry fired alone"
        );
        assert_eq!(tick(&mut dev, 500_000), "$A,12.34*47\r\n");
    }

    /// One byte per poll, and the clock keeps running while the queue drains —
    /// the difference from the hand-written NEO-6M, stated as a test.
    #[test]
    fn the_stream_clock_does_not_stop_while_the_queue_drains() {
        let mut dev = stream();
        // Credit 500 ms in ONE poll: the sentence is queued and the first byte
        // comes back.
        assert_eq!(dev.poll(500_000), Some(b'$'));
        // Now drain with zero credit; the timer must not fire again.
        assert_eq!(drain(&mut dev), "A,12.34*47\r\n");
        // Another 500 ms lands the NEXT sentence on the true grid, regardless
        // of how long the first took to clock out.
        assert_eq!(tick(&mut dev, 500_000), "$B*42\r\n");
    }

    #[test]
    fn a_stimulus_channel_reaches_the_template() {
        let mut dev = stream();
        SimInput::set_input(&mut dev, "x", 1.5).expect("x is a declared channel");
        assert_eq!(tick(&mut dev, 500_000), "$A,01.50*47\r\n");
    }

    const DELAYED: &str = r#"
type: test_uart_delay
behavior:
  primitive: uart_device
  uart:
    frames: { terminator: "\r\n" }
    responses:
      - { match: any, respond: "OK\r\n", delay_us: 90000 }
"#;

    #[test]
    fn a_delayed_answer_waits_its_declared_microseconds() {
        let desc = DeviceDescriptor::from_yaml(DELAYED).unwrap();
        validate_descriptor(&desc).unwrap();
        let mut dev =
            DeclarativeUartDevice::new("m".into(), &desc, std::borrow::Cow::Borrowed(&[])).unwrap();
        send(&mut dev, "AT\r\n");
        assert_eq!(dev.poll(0), None, "nothing yet");
        assert_eq!(dev.poll(89_999), None, "still inside the delay");
        assert_eq!(dev.poll(1), Some(b'O'), "the delay expired");
    }

    const STATEFUL: &str = r#"
type: test_uart_stateful
behavior:
  primitive: uart_device
  states: [command, data]
  uart:
    frames: { terminator: "\r\n" }
    responses:
      - { match: "AT+MODE=1", respond: "OK\r\n", do: [ { goto: data } ] }
      - { match: any, respond: "ERROR\r\n" }
"#;

    #[test]
    fn a_response_action_changes_the_parts_state() {
        let desc = DeviceDescriptor::from_yaml(STATEFUL).unwrap();
        validate_descriptor(&desc).unwrap();
        let mut dev =
            DeclarativeUartDevice::new("s".into(), &desc, std::borrow::Cow::Borrowed(&[])).unwrap();
        assert_eq!(dev.rule_machine().state(), "command");
        send(&mut dev, "AT+MODE=1\r\n");
        assert_eq!(drain(&mut dev), "OK\r\n");
        assert_eq!(dev.rule_machine().state(), "data");
    }

    #[test]
    fn a_descriptor_with_no_uart_block_is_refused() {
        let desc =
            DeviceDescriptor::from_yaml("type: t\nbehavior:\n  primitive: uart_device\n").unwrap();
        let err = validate_descriptor(&desc).unwrap_err();
        assert!(format!("{err:#}").contains("no `uart:` block"), "{err:#}");
    }

    #[test]
    fn a_stream_part_that_also_declares_registers_is_refused() {
        let desc = DeviceDescriptor::from_yaml(
            r#"
type: t
behavior:
  primitive: uart_device
  i2c: { default_address: 0x10 }
  uart:
    responses: [ { match: any, respond: "OK" } ]
"#,
        )
        .unwrap();
        let err = validate_descriptor(&desc).unwrap_err();
        assert!(format!("{err:#}").contains("no registers"), "{err:#}");
    }
}
