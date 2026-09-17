// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **declarative SPI device** — one engine device driven entirely by a
//! datasheet-shaped [`labwired_config::SpiSpec`], so a register-style SPI sensor
//! that fits the CS-framed command/register shape is a YAML file with zero Rust.
//!
//! The wire model is the near-universal register-sensor framing (ADXL345,
//! BMP280-SPI, LIS3DH): CS↓, one command byte carrying a read/write bit and a
//! register address, then a streamed word; a multi-byte burst auto-increments
//! the address. A read-only part (`command_bytes: 0`, e.g. MAX31855) clocks its
//! register-0 word straight out on CS↓. The measurement→word math is shared with
//! the I²C engine via [`super::declarative_regs`]; only the framing is new.

use std::any::Any;
use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use labwired_config::{
    DeviceDescriptor, Event, FrameSpec, RegisterAccess, RegisterSpec, SpiFraming,
};

use super::declarative_regs::{
    apply_timing_action, apply_write, encode_raw, leak_labs, read_clears, register_read_bytes,
    unpack, validate_timers, TimerBank,
};
use super::rule_machine::{RuleCtx, RuleMachine};
use crate::peripherals::spi::{SpiDevice, SpiSampling};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

pub struct GenericSpiDevice {
    cs_pin: String,
    framing: SpiFraming,
    registers: Vec<RegisterSpec>,

    slots: HashMap<String, f64>,
    reg_values: HashMap<String, u32>,

    // Per-frame state.
    cmd_consumed: u8,
    is_read: Option<bool>,
    cur_addr: Option<u16>,
    read_buf: Vec<u8>,
    /// Name of the register whose word ENDS at each `read_buf` index, so a
    /// burst can apply `on_read` at the moment each register's last byte
    /// leaves — the same "the read completed" point the I²C auto-increment
    /// path uses. Empty unless some register declares `on_read`.
    read_ends: Vec<Option<String>>,
    read_idx: usize,
    latched: bool,
    /// Explicit `cs_select()` is active. Soft-CS auto-restart must not fire
    /// while CS is held, or past-end bytes re-latch the same word (0x00) instead
    /// of returning 0xFF as an open bus.
    cs_held: bool,
    /// Bytes accumulated toward the current write register's width.
    write_acc: Vec<u8>,

    channels: &'static [InputChannel],
    component_id: Option<String>,
    /// How this instance latches the wire. [`SpiSampling::Byte`] unless the
    /// lab asked for edge-accurate sampling (`config.spi_mode`), so every
    /// existing manifest keeps the byte-level path it always had.
    sampling: SpiSampling,
    /// Simulated microseconds this device has been told have elapsed, summed
    /// from [`SpiDevice::advance_time_us`].
    ///
    /// Phase A wired the CLOCK; Phase B is what reads it — `timers` below ages
    /// on this counter exactly the way `declarative_i2c`'s `elapsed_us` does.
    /// A device that declares no timer still reads nothing from it, so its
    /// transcript is byte-for-byte what it was
    /// (`declarative_device_byte_parity` is the proof).
    elapsed_us: u64,
    /// Free-running device timers (`behavior.timers`). Empty ⇒ every timer
    /// code path short-circuits, so a device without one is unchanged.
    timers: TimerBank,

    /// **Tier 2**: states, variables, FIFOs and output pins. `None` ⇒ the
    /// descriptor declares none, and every rule path short-circuits. It holds
    /// no timer state: `timers` above is the ONE clock.
    rules: Option<RuleMachine>,
    /// Message framing, when the part declares any.
    frames: Option<FrameSpec>,
    /// MOSI bytes clocked since the last `frame` event, for a [`FrameSpec`]
    /// with a fixed `length`.
    frame_bytes: u16,
}

/// Validate the static descriptor contract for the `spi_device` primitive.
///
/// This stays separate from construction so manifest preflight can reject
/// malformed unused packs without allocating runtime input tables.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    if descriptor.behavior.primitive != "spi_device" {
        bail!(
            "declarative spi kit requires behavior.primitive: spi_device, got '{}'",
            descriptor.behavior.primitive
        );
    }
    let spec = descriptor
        .behavior
        .spi
        .as_ref()
        .context("declarative spi device is missing behavior.spi")?;
    if spec.registers.is_empty() {
        bail!("behavior.spi declares no registers");
    }
    if spec.framing.command_bytes > 1 {
        bail!(
            "behavior.spi command_bytes {} unsupported (0 or 1)",
            spec.framing.command_bytes
        );
    }
    // `zero_when` power-gates work here too (the read math is shared with
    // the I²C engine), so a dangling reference must not silently no-op into
    // "gate never fires". No shipping SPI descriptor uses one yet; this is
    // the guard that keeps the first one from being wrong in silence.
    for reg in &spec.registers {
        if let Some(z) = &reg.zero_when {
            if !spec.registers.iter().any(|r| r.name == z.register) {
                bail!(
                    "register '{}' zero_when register '{}' is not a declared register",
                    reg.name,
                    z.register
                );
            }
            if z.mask == 0 {
                bail!(
                    "register '{}' zero_when mask is 0 — the gate could never fire",
                    reg.name
                );
            }
        }
        // The SPI command byte carries the address, so a 16-bit `addr` (which
        // exists for I²C `pointer_width: 2` parts) can never be selected here.
        // Rejecting it at load beats a register that silently answers nothing.
        if reg.addr > 0xFF {
            bail!(
                "register '{}' addr {:#06x} is out of range for an SPI command byte",
                reg.name,
                reg.addr
            );
        }
    }
    let names: Vec<String> = spec.registers.iter().map(|r| r.name.clone()).collect();
    validate_timers(
        &descriptor.behavior.timers,
        &names,
        &descriptor.behavior.rules,
    )?;
    // Tier 2: the same load-time strictness the I²C primitive applies — every
    // expression parses, every name a rule mentions is declared.
    labwired_config::compile_rules(&descriptor.behavior.rules)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    super::declarative_gpio::validate_rule_names(descriptor)?;
    Ok(())
}

impl GenericSpiDevice {
    pub fn from_descriptor(
        descriptor: &DeviceDescriptor,
        cs_pin: String,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        validate_descriptor(descriptor)?;
        let spec = descriptor
            .behavior
            .spi
            .as_ref()
            .context("declarative spi device is missing behavior.spi")?;
        let mut slots = HashMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
            }
        }
        let reg_values = spec
            .registers
            .iter()
            .map(|r| (r.name.clone(), r.reset))
            .collect();
        Ok(Self {
            cs_pin,
            framing: spec.framing.clone(),
            registers: spec.registers.clone(),
            slots,
            reg_values,
            cmd_consumed: 0,
            is_read: None,
            cur_addr: None,
            read_buf: Vec::new(),
            read_ends: Vec::new(),
            read_idx: 0,
            latched: false,
            cs_held: false,
            write_acc: Vec::with_capacity(4),
            channels,
            component_id: None,
            sampling: SpiSampling::Byte,
            elapsed_us: 0,
            timers: TimerBank::new(&descriptor.behavior.timers),
            rules: RuleMachine::from_behavior(&descriptor.behavior)?,
            frames: descriptor.behavior.frames.clone(),
            frame_bytes: 0,
        })
    }

    pub fn from_yaml(yaml: &str, cs_pin: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        let channels = super::declarative_i2c::leak_channels(&descriptor);
        Self::from_descriptor(&descriptor, cs_pin.to_string(), channels)
    }

    /// Strap this instance for edge-accurate sampling in the given SPI mode
    /// (0..=3, bit 1 = CPOL, bit 0 = CPHA) — the part's own clock mode, as its
    /// datasheet states it. Without this call the device stays on the default
    /// byte-level path and never sees a clock edge.
    pub fn set_spi_mode(&mut self, mode: u8) -> Result<()> {
        if mode > 3 {
            bail!("spi_mode {mode} is not a SPI mode (0..=3)");
        }
        self.sampling = SpiSampling::edge_mode(mode);
        Ok(())
    }

    fn find_register(&self, addr: u16) -> Option<&RegisterSpec> {
        self.registers.iter().find(|r| r.addr == addr)
    }

    /// The register whose byte span covers `addr`, which is not always the one
    /// whose base equals it — see `build_read_buf`.
    fn find_register_containing(&self, addr: u16) -> Option<&RegisterSpec> {
        self.registers
            .iter()
            .find(|r| addr >= r.addr && addr < r.addr.saturating_add(u16::from(r.width)))
    }

    fn next_addr_above(&self, addr: u16) -> Option<u16> {
        self.registers
            .iter()
            .filter(|r| r.addr > addr)
            .map(|r| r.addr)
            .min()
    }

    /// Concatenated read stream from `start`: every register at addr ≥ start in
    /// ascending order (auto-increment), or just the matched register.
    /// Bytes a read beginning at `start` streams back.
    ///
    /// A read may begin *inside* a multi-byte register, because a datasheet
    /// numbers every byte of one as its own address: the ADXL345 declares
    /// DATAX0 at 0x32 and DATAX1 at 0x33 and says the low byte need not be read
    /// when it is not wanted, so pointing at 0x33 must return the high byte.
    ///
    /// Selecting on `addr >= start` did not do that. It skipped the register
    /// containing `start` and answered from the *next* one, so a firmware that
    /// read one axis byte on its own got a different register's value —
    /// 0x37 returned FIFO_CTL — with nothing to distinguish it from real data.
    /// Matching on the span and dropping the bytes before `start` serves the
    /// address the caller actually named.
    fn build_read_buf(&self, start: u16) -> (Vec<u8>, Vec<Option<String>>) {
        let mut out = Vec::new();
        // Which register's word ENDS at each byte index — the hook `on_read`
        // needs, and nothing else. Left empty when no register declares one, so
        // a device without the field allocates nothing extra.
        let track_ends = self.registers.iter().any(read_clears);
        let mut ends: Vec<Option<String>> = Vec::new();
        let mut push = |r: &RegisterSpec, bytes: Vec<u8>, out: &mut Vec<u8>| {
            let n = bytes.len();
            out.extend(bytes);
            if track_ends {
                ends.resize(out.len(), None);
                if n > 0 && read_clears(r) {
                    let last = out.len() - 1;
                    ends[last] = Some(r.name.clone());
                }
            }
        };
        if self.framing.auto_increment {
            let mut regs: Vec<&RegisterSpec> = self
                .registers
                .iter()
                .filter(|r| r.addr.saturating_add(u16::from(r.width)) > start)
                .collect();
            regs.sort_by_key(|r| r.addr);
            for r in regs {
                let skip = usize::from(start.saturating_sub(r.addr));
                let bytes: Vec<u8> = register_read_bytes(r, &self.slots, &self.reg_values)
                    .into_iter()
                    .skip(skip)
                    .collect();
                push(r, bytes, &mut out);
            }
        } else if let Some(r) = self.find_register_containing(start) {
            let skip = usize::from(start - r.addr);
            let bytes: Vec<u8> = register_read_bytes(r, &self.slots, &self.reg_values)
                .into_iter()
                .skip(skip)
                .collect();
            push(r, bytes, &mut out);
        }
        (out, ends)
    }

    /// Current engineering-unit value of a SimInput stimulus channel (the value
    /// last set via `set_input`, or the descriptor's declared default). Returns
    /// `None` if the device has no such channel. Lets consumers (e.g. the wasm
    /// inspector) read a declarative device's state without a concrete-type downcast.
    pub fn input_value(&self, key: &str) -> Option<f64> {
        self.slots.get(key).copied()
    }

    /// Simulated microseconds this device has been told have elapsed. Read by
    /// tests that prove the central device-time drive actually reaches SPI.
    pub fn elapsed_us(&self) -> u64 {
        self.elapsed_us
    }
}

// ─── Tier 2: the rule machine's view of this device ────────────────────────

/// The [`RuleCtx`] a declarative SPI device hands its [`RuleMachine`]. Same
/// shape as the I²C one — the rule vocabulary is transport-agnostic on purpose,
/// so the SAME `rules:` block ports between an I²C and a SPI variant of a part
/// (ADXL345 is both) without a word changing.
struct SpiRuleCtx<'a> {
    registers: &'a [RegisterSpec],
    reg_values: &'a mut HashMap<String, u32>,
    slots: &'a HashMap<String, f64>,
}

impl RuleCtx for SpiRuleCtx<'_> {
    fn reg(&self, name: &str) -> Option<u32> {
        self.reg_values.get(name).copied()
    }
    fn set_reg(&mut self, name: &str, value: u32) {
        self.reg_values.insert(name.to_string(), value);
    }
    fn field_bits(&self, register: &str, field: &str) -> Option<(u8, u32)> {
        let reg = self.registers.iter().find(|r| r.name == register)?;
        let f = reg.bits.iter().find(|b| b.name == field)?;
        Some((f.shift, f.mask()))
    }
    fn input(&self, key: &str) -> i64 {
        let raw = self.slots.get(key).copied().unwrap_or(0.0);
        match self
            .registers
            .iter()
            .find(|r| r.source.as_deref() == Some(key))
        {
            Some(reg) => {
                let encoded = encode_raw(
                    raw,
                    reg.encode.as_ref(),
                    reg.source_scale.unwrap_or(1.0),
                    reg.width,
                    reg.signed,
                );
                if reg.signed {
                    let bits = 8 * u32::from(reg.width);
                    if bits < 32 && encoded & (1 << (bits - 1)) != 0 {
                        return i64::from(encoded as i32 | !((1i32 << bits) - 1));
                    }
                }
                i64::from(encoded)
            }
            None => raw as i64,
        }
    }
}

impl GenericSpiDevice {
    /// Raise a Tier-2 event; no-op for a descriptor with no rules.
    fn raise(&mut self, event: Event, written: i64) {
        let Some(mut machine) = self.rules.take() else {
            return;
        };
        {
            let mut ctx = SpiRuleCtx {
                registers: &self.registers,
                reg_values: &mut self.reg_values,
                slots: &self.slots,
            };
            machine.fire(&event, written, &mut ctx);
        }
        self.rules = Some(machine);
    }

    /// Fire a Tier-2 event and immediately apply any `timer:` action it queued.
    /// See the I²C twin for why the timer drive uses bare `raise` instead.
    fn raise_and_settle(&mut self, event: Event, written: i64) {
        self.raise(event, written);
        self.drain_timer_requests();
    }

    /// Let the rule machine record the elapsed µs. It schedules nothing: the
    /// device's [`TimerBank`] is the one clock and raises `Event::Timer`.
    fn advance_rule_time(&mut self, us: u64) {
        if let Some(m) = self.rules.as_mut() {
            m.advance_time_us(us);
        }
    }

    /// Apply whatever `timer:` actions the rules queued to the ONE bank.
    fn drain_timer_requests(&mut self) {
        let Some(m) = self.rules.as_mut() else { return };
        let requests = m.take_timer_requests();
        if requests.is_empty() {
            return;
        }
        for (name, start) in requests {
            if start {
                self.timers.start_named(&name, self.elapsed_us);
            } else {
                self.timers.stop_named(&name);
            }
        }
    }

    /// Read-only view of the rule machine, for tests and diagnostics.
    pub fn rule_machine(&self) -> Option<&RuleMachine> {
        self.rules.as_ref()
    }
}

impl SpiDevice for GenericSpiDevice {
    fn sampling(&self) -> SpiSampling {
        self.sampling
    }

    /// Record elapsed simulated time and age the part's own timers on it.
    /// A device that declares none is untouched.
    ///
    /// ONE clock, two consumers: each due timer runs its `on_fire` register
    /// actions and then raises a Tier-2 `timer:<name>` event, in that order, so
    /// a rule sees the registers the same firing already changed. The rule
    /// machine holds no timer state of its own — it could not drift from this
    /// one if it tried.
    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(us);
        self.advance_rule_time(us);
        if !self.timers.is_empty() {
            for (name, actions) in self.timers.due_by_timer(self.elapsed_us) {
                for action in &actions {
                    apply_timing_action(action, &mut self.reg_values);
                }
                self.raise(Event::Timer { name }, 0);
            }
        }
    }

    /// Tier 2: hand the bus whatever pin transitions the rules queued.
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        match self.rules.as_mut() {
            Some(m) => m.take_pin_drives(),
            None => Vec::new(),
        }
    }

    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.cmd_consumed = 0;
        self.is_read = None;
        self.cur_addr = None;
        self.read_buf.clear();
        self.read_ends.clear();
        self.read_idx = 0;
        self.latched = false;
        self.cs_held = true;
        self.write_acc.clear();
        if self.framing.command_bytes == 0 {
            self.is_read = Some(true);
            self.cur_addr = Some(0);
        }
        self.raise_and_settle(Event::CsSelect, 0);
    }

    fn cs_release(&mut self) {
        self.cs_held = false;
        self.write_acc.clear();
        self.raise_and_settle(Event::CsRelease, 0);
        // CS↑ always closes a frame, the SPI twin of the I²C STOP: a short
        // message is delivered rather than swallowed.
        if self.frames.is_some() {
            self.frame_bytes = 0;
            self.raise_and_settle(Event::Frame, 0);
        }
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // Framing: close the frame the moment the declared length is clocked,
        // without waiting for CS↑. Same contract as the I²C side.
        let mut close_frame = false;
        if let Some(length) = self.frames.as_ref().and_then(|f| f.length) {
            if length > 0 {
                self.frame_bytes = self.frame_bytes.saturating_add(1);
                if self.frame_bytes >= length {
                    self.frame_bytes = 0;
                    close_frame = true;
                }
            }
        }
        let miso = self.transfer_inner(mosi);
        if close_frame {
            self.raise_and_settle(Event::Frame, i64::from(mosi));
        }
        miso
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl GenericSpiDevice {
    /// The wire exchange itself, split out so the framing counter above can
    /// raise `frame` once the byte has been processed.
    fn transfer_inner(&mut self, mosi: u8) -> u8 {
        // Soft-CS / matrix path: when CS was never held (or has been released),
        // enter the read-only data phase and re-frame after a full word so a
        // CS-high dummy flush does not permanently desync multi-byte reads.
        // While CS is held, past-end bytes stay 0xFF (open bus) — do not re-latch.
        if self.framing.command_bytes == 0 && !self.cs_held {
            let need_start =
                self.is_read.is_none() || (self.latched && self.read_idx >= self.read_buf.len());
            if need_start {
                // Soft-CS synthetic select: frame start without claiming hard CS.
                self.cmd_consumed = 0;
                self.is_read = Some(true);
                self.cur_addr = Some(0);
                self.read_buf.clear();
                self.read_idx = 0;
                self.latched = false;
                self.write_acc.clear();
            }
        }
        // Command phase.
        if self.framing.command_bytes > 0 && self.cmd_consumed < self.framing.command_bytes {
            self.cmd_consumed += 1;
            if self.cmd_consumed == self.framing.command_bytes {
                if let Some(bit) = self.framing.rw_bit {
                    let set = (mosi >> bit) & 1 == 1;
                    self.is_read = Some(set == self.framing.rw_read_high);
                }
                self.cur_addr = Some(u16::from(
                    (mosi >> self.framing.addr_shift) & self.framing.addr_mask,
                ));
            }
            return 0x00;
        }
        // Data phase.
        let addr = self.cur_addr.unwrap_or(0);
        // Writes require an explicit rw_bit in the framing; a part with rw_bit: None never leaves is_read == None, so every data byte is a read.
        let write = matches!(self.is_read, Some(false));
        if write {
            self.write_acc.push(mosi);
            // The completed write is computed under a CLONED register so the
            // Tier-2 event below can take `&mut self`.
            let completed: Option<(String, u32)> = match self.find_register(addr).cloned() {
                Some(reg)
                    if reg.access == RegisterAccess::Rw
                        && self.write_acc.len() == reg.width as usize =>
                {
                    let written = unpack(&self.write_acc, reg.endian);
                    // `write_mask` (shared with the I²C engine) keeps the bits
                    // silicon owns; absent ⇒ the whole word is replaced. What
                    // the writable bits then DO is `on_write` — a plain store
                    // unless the datasheet says otherwise.
                    let prev = self.reg_values.get(&reg.name).copied().unwrap_or(0);
                    let val = apply_write(&reg, prev, written);
                    self.reg_values.insert(reg.name.clone(), val);
                    if !self.timers.is_empty() {
                        self.timers.start_on_write(&reg.name, val, self.elapsed_us);
                    }
                    self.write_acc.clear();
                    if self.framing.auto_increment {
                        if let Some(next) = self.next_addr_above(addr) {
                            self.cur_addr = Some(next);
                        }
                    }
                    Some((reg.name.clone(), written))
                }
                _ => None,
            };
            if let Some((name, written)) = completed {
                // Tier 2 LAST, so a rule sees the post-write register.
                self.raise_and_settle(
                    Event::Write {
                        register: name,
                        field: None,
                    },
                    i64::from(written),
                );
            }
            return 0x00;
        }
        // Read.
        if !self.latched {
            let (buf, ends) = self.build_read_buf(addr);
            self.read_buf = buf;
            self.read_ends = ends;
            self.latched = true;
            // The read event fires as the word LATCHES, matching the I²C side.
            if let Some(name) = self.find_register(addr).map(|r| r.name.clone()) {
                self.raise_and_settle(Event::Read { register: name }, 0);
            }
        }
        let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
        // `on_read: clear` fires as the register's LAST byte leaves — the burst
        // analogue of the I²C auto-increment rule (see `RegisterSpec::on_read`).
        if let Some(Some(name)) = self.read_ends.get(self.read_idx).cloned() {
            self.reg_values.insert(name, 0);
        }
        self.read_idx += 1;
        byte
    }
}

impl SimInput for GenericSpiDevice {
    fn input_channels(&self) -> &'static [InputChannel] {
        self.channels
    }
    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.slots.insert(key.to_string(), value);
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }
    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

/// A [`PeripheralKit`] backed by a declarative `spi_device` descriptor — one
/// instance per YAML device. Phase 1 registers no real parts, so nothing is
/// added to `registry::KITS` and the offline peripherals manifest is unchanged.
pub struct DeclarativeSpiKit {
    descriptor: DeviceDescriptor,
    channels: &'static [InputChannel],
    metadata: &'static KitMetadata,
}

impl DeclarativeSpiKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;
        let channels = super::declarative_i2c::leak_channels(&descriptor);
        let metadata = leak_metadata(&descriptor, channels);
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }
}

fn leak_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
) -> &'static KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative SPI device.".to_string());
    let config_keys: &'static [ConfigKey] = Box::leak(
        vec![
            ConfigKey {
                name: "cs_pin",
                ty: ConfigType::Str,
                doc: "CS GPIO pin wired as SPI chip-select (e.g. \"PA4\").",
            },
            ConfigKey {
                name: "spi_mode",
                ty: ConfigType::Int,
                doc: "Opt in to edge-accurate (bit-level) slave sampling in this SPI mode (0..=3). Omit for the default byte-level frame exchange.",
            },
        ]
        .into_boxed_slice(),
    );
    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary.clone()),
        detail: leak(summary),
        transport: Transport::Spi,
        category: Category::Spi,
        config_keys,
        labs: leak_labs(
            descriptor
                .metadata
                .as_ref()
                .map(|m| m.labs.as_slice())
                .unwrap_or(&[]),
        ),
        inputs: channels,
    }))
}

impl PeripheralKit for DeclarativeSpiKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut device =
            GenericSpiDevice::from_descriptor(&self.descriptor, cs_pin, self.channels)?;
        // Opt-in, per lab: `config: { spi_mode: N }` straps this part for
        // edge-accurate sampling in ITS mode, so a controller programmed for a
        // different CPOL/CPHA corrupts the exchange the way silicon does.
        // Absent — which is every manifest that exists today — leaves the
        // byte-level path untouched.
        if let Some(mode) = ctx.config_i64("spi_mode") {
            let m = u8::try_from(mode)
                .map_err(|_| anyhow::anyhow!("spi_mode {mode} is not a SPI mode (0..=3)"))?;
            device.set_spi_mode(m)?;
        }
        // Tier 2: bind `outputs:` roles to pads (the DRDY/IRQ twin of the I²C
        // INT line) before the device goes in.
        ctx.bind_output_pins(&self.descriptor)?;
        ctx.attach_spi_device(Box::new(device))
    }
}

// ─── Registry statics ──────────────────────────────────────────────────────
//
// A `DeclarativeSpiKit` is parsed from YAML at runtime, but the registry
// (`registry::KITS`) is a const slice of `&'static dyn PeripheralKit`. A
// `static LazyLock<DeclarativeSpiKit>` is the const-initialisable cell that
// bridges the two: the descriptor is parsed once on first access, and the
// `PeripheralKit` impl below forwards through it. Real parts get one static
// each here and one line in `registry::KITS`; the descriptor lives entirely in
// `configs/devices/*.yaml`.

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeSpiKit> {
    fn metadata(&self) -> &'static KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Analog Devices ADXL345 accelerometer (declarative `adxl345_spi.yaml`).
pub static ADXL345_KIT: LazyLock<DeclarativeSpiKit> = LazyLock::new(|| {
    DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("adxl345_spi")
            .expect("adxl345_spi descriptor embedded"),
    )
    .expect("adxl345_spi.yaml is a valid declarative spi descriptor")
});

/// Maxim MAX31855 thermocouple converter (declarative `max31855.yaml`).
pub static MAX31855_KIT: LazyLock<DeclarativeSpiKit> = LazyLock::new(|| {
    DeclarativeSpiKit::from_yaml(
        labwired_config::embedded_device_yaml("max31855").expect("max31855 descriptor embedded"),
    )
    .expect("max31855.yaml is a valid declarative spi descriptor")
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    const FIXTURE: &str = include_str!("declarative_spi_fixture.yaml");

    fn dev() -> GenericSpiDevice {
        GenericSpiDevice::from_yaml(FIXTURE, "PA4").unwrap()
    }

    /// Clock a read: assert CS, send command byte (read | addr), read `n` bytes.
    fn read_reg(d: &mut GenericSpiDevice, addr: u8, n: usize) -> Vec<u8> {
        d.cs_select();
        d.transfer(0x80 | addr); // rw_bit=7 set ⇒ read
        let out: Vec<u8> = (0..n).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        out
    }

    /// Clock a write: assert CS, send command byte (write | addr), send data.
    fn write_reg(d: &mut GenericSpiDevice, addr: u8, data: &[u8]) {
        d.cs_select();
        d.transfer(addr); // rw_bit=7 clear ⇒ write
        for &b in data {
            d.transfer(b);
        }
        d.cs_release();
    }

    #[test]
    fn cs_pin_is_wired() {
        assert_eq!(dev().cs_pin(), "PA4");
    }

    #[test]
    fn whoami_reads_fixed_reset_value() {
        let mut d = dev();
        assert_eq!(read_reg(&mut d, 0x00, 1), vec![0xE5]);
    }

    #[test]
    fn data_register_sources_measurement_little_endian() {
        // accel_x default 1 g × 256 LSB/g × range×1 = 256 = 0x0100, LE.
        let mut d = dev();
        assert_eq!(read_reg(&mut d, 0x32, 2), vec![0x00, 0x01]);
    }

    #[test]
    fn set_input_drives_the_data_register() {
        let mut d = dev();
        d.set_input("accel_x", 2.0).unwrap();
        // 2 g × 256 = 512 = 0x0200, LE.
        assert_eq!(read_reg(&mut d, 0x32, 2), vec![0x00, 0x02]);
    }

    #[test]
    fn rw_register_write_then_scale_from_changes_data() {
        let mut d = dev();
        // Program RANGE = 3 ⇒ scale_from ×4 ⇒ 1 g × 256 × 4 = 1024 = 0x0400.
        write_reg(&mut d, 0x31, &[0x03]);
        assert_eq!(read_reg(&mut d, 0x32, 2), vec![0x00, 0x04]);
    }

    #[test]
    fn auto_increment_walks_ascending_registers() {
        // Read starting at 0x31 for 3 bytes: RANGE(1B, reset 0) then DATAX(2B).
        let mut d = dev();
        let b = read_reg(&mut d, 0x31, 3);
        assert_eq!(b, vec![0x00, 0x00, 0x01]); // RANGE=0, then DATAX 256 LE
    }

    #[test]
    fn reads_past_the_last_register_return_ff() {
        let mut d = dev();
        let b = read_reg(&mut d, 0x32, 4); // DATAX is 2 bytes; 2 more ⇒ 0xFF
        assert_eq!(&b[2..], &[0xFF, 0xFF]);
    }

    #[test]
    fn out_of_range_and_unknown_channels_are_rejected() {
        let mut d = dev();
        assert!(d.set_input("accel_x", 99.0).is_err());
        assert!(d.set_input("nope", 1.0).is_err());
    }

    #[test]
    fn declarative_spi_kit_builds_metadata_from_descriptor() {
        let kit = DeclarativeSpiKit::from_yaml(FIXTURE).unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "test_spi_fixture");
        assert!(matches!(
            m.transport,
            crate::peripherals::kit::Transport::Spi
        ));
        assert_eq!(m.inputs.len(), 1);
        assert!(m.inputs.iter().any(|c| c.key == "accel_x"));
        // No `metadata.labs` in FIXTURE ⇒ no labs advertised.
        assert_eq!(m.labs.len(), 0);
    }

    const LABS_FIXTURE: &str = r#"
type: test_spi_labs_fixture

behavior:
  primitive: spi_device
  spi:
    framing:
      command_bytes: 0
    registers:
      - name: TEMP
        addr: 0
        width: 4
        endian: be
        access: r
        source: temperature
        encode: { scale: 1.0 }

metadata:
  label: "Declarative SPI labs fixture"
  summary: "Test-only fixture asserting metadata.labs round-trips."
  category: spi
  inputs:
    - { key: temperature, label: "Temperature", unit: "°C", min: -50, max: 200, default: 100 }
  labs:
    - board_id: "test-board-lab"
      chip: "stm32f103"
      example_dir: "spi_test_fixture"
      demo_elf: "spi_test_fixture.elf"
"#;

    #[test]
    fn declarative_spi_kit_advertises_labs_from_descriptor_metadata() {
        let kit = DeclarativeSpiKit::from_yaml(LABS_FIXTURE).unwrap();
        let labs = kit.metadata().labs;
        assert_eq!(labs.len(), 1);
        assert_eq!(labs[0].board_id, "test-board-lab");
        assert_eq!(labs[0].chip, "stm32f103");
        assert_eq!(labs[0].example_dir, "spi_test_fixture");
        assert_eq!(labs[0].demo_elf, "spi_test_fixture.elf");
    }

    /// A MAX31855-style read-only part: `command_bytes: 0` means CS↓ clocks
    /// register 0 straight out with no leading command byte.
    const READ_ONLY_FIXTURE: &str = r#"
type: test_spi_readonly_fixture

behavior:
  primitive: spi_device
  spi:
    framing:
      command_bytes: 0
    registers:
      - name: TEMP
        addr: 0
        width: 4
        endian: be
        access: r
        source: temperature
        encode: { scale: 1.0 }

metadata:
  label: "Declarative SPI read-only fixture"
  summary: "Test-only MAX31855-shaped read-only SPI part (command_bytes: 0)."
  category: spi
  inputs:
    - { key: temperature, label: "Temperature", unit: "°C", min: -50, max: 200, default: 100 }
"#;

    fn read_only_dev() -> GenericSpiDevice {
        GenericSpiDevice::from_yaml(READ_ONLY_FIXTURE, "PA4").unwrap()
    }

    #[test]
    fn command_bytes_zero_reads_register_zero_straight_off_cs_select() {
        let mut d = read_only_dev();
        d.cs_select();
        // Default temperature 100.0 × scale 1.0 = 100 = 0x00000064, big-endian.
        let bytes: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x64]);
        // A byte past the 4-byte register width returns 0xFF.
        assert_eq!(d.transfer(0x00), 0xFF);
        d.cs_release();
    }

    #[test]
    fn command_bytes_zero_reasserts_the_same_word_on_a_second_transaction() {
        let mut d = read_only_dev();
        d.cs_select();
        let first: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        // Re-select for a second transaction: frame state must reset so the
        // same register-0 word is clocked out again from the start.
        d.cs_select();
        let second: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(first, vec![0x00, 0x00, 0x00, 0x64]);
        assert_eq!(second, vec![0x00, 0x00, 0x00, 0x64]);
    }

    #[test]
    fn declarative_spi_kit_rejects_wrong_primitive() {
        let yaml = r#"
type: bad
behavior:
  primitive: i2c_device
  spi:
    registers:
      - { name: A, addr: 0, width: 1, endian: le, access: r }
"#;
        assert!(DeclarativeSpiKit::from_yaml(yaml).is_err());
    }

    #[test]
    fn adxl345_kit_reads_devid_and_signed_axis() {
        let kit = DeclarativeSpiKit::from_yaml(
            labwired_config::embedded_device_yaml("adxl345_spi").unwrap(),
        )
        .unwrap();
        assert_eq!(kit.metadata().device_type, "adxl345_spi");
        // Build the device and read DEVID + a negative Z.
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("adxl345_spi").unwrap(),
            "PA4",
        )
        .unwrap();
        d.cs_select();
        d.transfer(0x80); // read DEVID (0x00)
        assert_eq!(d.transfer(0x00), 0xE5);
        d.cs_release();
        d.set_input("accel_z", -1.0).unwrap();
        d.cs_select();
        d.transfer(0x80 | 0x36); // read DATAZ0
        let lo = d.transfer(0x00);
        let hi = d.transfer(0x00);
        d.cs_release();
        assert_eq!(u16::from_le_bytes([lo, hi]), 0xFF00); // -256 two's-complement
    }

    #[test]
    fn max31855_reads_composite_frame_no_command() {
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("max31855").unwrap(),
            "PA4",
        )
        .unwrap();
        d.set_input("temperature", 100.0).unwrap(); // 400 = 0x190 @ [31:18]
        d.set_input("internal", 25.0).unwrap(); // 400 = 0x190 @ [15:4]
        d.cs_select(); // command_bytes:0 → data phase immediately
        let b: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(b, vec![0x06, 0x40, 0x19, 0x00]);
        // A negative thermocouple reading sets the sign bits.
        d.set_input("temperature", -25.0).unwrap();
        d.cs_select();
        let n: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        let word = u32::from_be_bytes([n[0], n[1], n[2], n[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C); // -100 in 14-bit two's-complement
    }

    /// Parity anchor: the declarative descriptor must reproduce the
    /// hand-written `Max31855` model's default power-on frame and stimulus
    /// response byte-for-byte. Default word = (100<<18)|(352<<4) = 0x01901600
    /// (tc=25.0°C, internal=22.0°C, fault=0 — see components/max31855.rs).
    #[test]
    fn max31855_parity() {
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("max31855").unwrap(),
            "PA4",
        )
        .unwrap();

        // Default frame: tc=25.0 -> 100<<18, internal=22.0 -> 352<<4.
        d.cs_select();
        let default_frame: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(default_frame, vec![0x01, 0x90, 0x16, 0x00]);

        // After driving both stimuli: tc=100.0 -> 400<<18, internal=25.0 -> 400<<4.
        d.set_input("temperature", 100.0).unwrap();
        d.set_input("internal", 25.0).unwrap();
        d.cs_select();
        let driven_frame: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        assert_eq!(driven_frame, vec![0x06, 0x40, 0x19, 0x00]);

        // Negative thermocouple reading: -100 in 14-bit two's-complement.
        d.set_input("temperature", -25.0).unwrap();
        d.cs_select();
        let neg: Vec<u8> = (0..4).map(|_| d.transfer(0x00)).collect();
        d.cs_release();
        let word = u32::from_be_bytes([neg[0], neg[1], neg[2], neg[3]]);
        assert_eq!((word >> 18) & 0x3FFF, 0x3F9C);
    }

    // ─── Tier 1 on SPI: side effects and timers ────────────────────────────
    //
    // The vocabulary is shared with the I²C engine (`declarative_regs`), so
    // these assert that the SPI FRAMING reaches it — a burst read clearing the
    // right register at the right byte, a command-byte write running the
    // datasheet action, a timer aging on the SPI `advance_time_us` hook.

    const TIER1_FIXTURE: &str = r#"
type: test_spi_tier1_fixture
behavior:
  primitive: spi_device
  timers:
    - name: sample
      period_us: 1000
      start: on_reset
      on_fire:
        - set_bits: { register: STATUS, bits: 0x08 }
  spi:
    framing: { command_bytes: 1, rw_bit: 7, rw_read_high: true, addr_mask: 0x3F, auto_increment: true }
    registers:
      - { name: STATUS, addr: 0x00, width: 1, endian: be, access: r, reset: 0x80, on_read: clear }
      - { name: DATA, addr: 0x01, width: 2, endian: be, access: r, reset: 0xBEEF }
      - { name: INT_ACK, addr: 0x03, width: 1, endian: be, access: rw, reset: 0x0F, on_write: one_to_clear }
"#;

    fn tier1() -> GenericSpiDevice {
        GenericSpiDevice::from_yaml(TIER1_FIXTURE, "PA4").unwrap()
    }

    #[test]
    fn spi_on_read_clear_zeroes_the_register_after_its_read() {
        let mut d = tier1();
        assert_eq!(read_reg(&mut d, 0x00, 1), vec![0x80], "the pre-clear value");
        assert_eq!(read_reg(&mut d, 0x00, 1), vec![0x00], "cleared by the read");
    }

    #[test]
    fn spi_on_read_clear_fires_at_the_registers_last_byte_of_a_burst() {
        // A burst from 0x00 streams STATUS then DATA. STATUS must clear when
        // ITS byte leaves — not when the burst was latched, and not when the
        // burst ends, or a status word could be zeroed under a master that is
        // still mid-read.
        let mut d = tier1();
        assert_eq!(read_reg(&mut d, 0x00, 3), vec![0x80, 0xBE, 0xEF]);
        assert_eq!(read_reg(&mut d, 0x00, 1), vec![0x00]);
    }

    #[test]
    fn spi_a_register_without_on_read_survives_a_burst() {
        // The negative control: DATA is read twice in the burst above and is
        // still there.
        let mut d = tier1();
        let _ = read_reg(&mut d, 0x00, 3);
        assert_eq!(read_reg(&mut d, 0x01, 2), vec![0xBE, 0xEF]);
    }

    #[test]
    fn spi_on_write_one_to_clear_clears_the_bits_written() {
        let mut d = tier1();
        write_reg(&mut d, 0x03, &[0x03]);
        assert_eq!(read_reg(&mut d, 0x03, 1), vec![0x0C], "0x0F & !0x03");
    }

    #[test]
    fn spi_timers_age_on_the_advance_time_hook() {
        // `SpiDevice::advance_time_us` is a default no-op on the trait, so a
        // controller that never calls it leaves the device frozen — the
        // documented holdout. Here it is called, so the part's own clock runs.
        let mut d = tier1();
        d.advance_time_us(999);
        assert_eq!(read_reg(&mut d, 0x00, 1), vec![0x80], "one µs short");
        let mut d = tier1();
        d.advance_time_us(1_000);
        assert_eq!(
            read_reg(&mut d, 0x00, 1),
            vec![0x88],
            "the sample bit is set"
        );
    }

    #[test]
    fn spi_a_device_with_no_timers_is_untouched_by_time() {
        let mut d = dev();
        let before = read_reg(&mut d, 0x00, 1);
        d.advance_time_us(10_000_000);
        assert_eq!(read_reg(&mut d, 0x00, 1), before);
    }

    #[test]
    fn spi_rejects_a_register_address_wider_than_the_command_byte() {
        let yaml = TIER1_FIXTURE.replace("addr: 0x03,", "addr: 0x0103,");
        let err = match GenericSpiDevice::from_yaml(&yaml, "PA4") {
            Ok(_) => panic!("a 16-bit SPI register address must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("out of range for an SPI command byte"),
            "got: {err}"
        );
    }

    #[test]
    fn input_value_reads_defaults_and_tracks_set_input() {
        let mut d = crate::peripherals::components::declarative_spi::GenericSpiDevice::from_yaml(
            labwired_config::embedded_device_yaml("max31855").unwrap(),
            "PA4",
        )
        .unwrap();

        assert_eq!(d.input_value("temperature"), Some(25.0));
        assert_eq!(d.input_value("internal"), Some(22.0));
        assert!(d.input_value("nope").is_none());

        d.set_input("temperature", 100.0).unwrap();
        assert_eq!(d.input_value("temperature"), Some(100.0));
    }
}
