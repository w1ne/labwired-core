// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic **declarative I²C device** — one engine device driven entirely by a
//! datasheet-shaped [`labwired_config::I2cSpec`], so a new I²C sensor that fits
//! the two covered wire-protocol shapes is a YAML file with zero Rust.
//!
//! The two shapes mirror the two hand-written reference families already in the
//! tree, and this device is byte-compatible with each:
//!   * **register-pointer** (`registers:`) — the master writes a 1-byte pointer
//!     then streams a fixed-width LE/BE word; rw registers accumulate + echo the
//!     master's writes. This is the VEML7700 protocol
//!     ([`super::veml7700`]).
//!   * **command** (`commands:`) — the master writes a 16-bit big-endian
//!     command, then reads N words each followed by a CRC-8 byte. This is the
//!     Sensirion protocol ([`super::scd41`] / [`super::sensirion`]).
//!
//! A descriptor is exactly one shape (registers XOR commands). Measurements are
//! externally driven through the ONE stimulus contract,
//! [`crate::sim_input::SimInput`]: `metadata.inputs` defines the channels, and
//! register/response `source:` keys read the current slot value and apply the
//! declared linear `encode` (+ optional register-bit-field `scale_from`). No
//! expression language, no per-device code — every YAML field is meaningful to
//! someone reading only the part datasheet.
//!
//! **Delay gating.** A command's `delay_us` gates its response on simulated
//! wall-clock, advanced through the [`crate::peripherals::i2c::I2cDevice::advance_time_us`]
//! hook — the same hook the trait documents ("a bus master that knows the
//! elapsed wall-clock calls this on a slave immediately before servicing it").
//! Of the shipping controllers only the nRF54L TWIM currently drives that hook,
//! so command devices with `delay_us` are faithful on that bus; the reference
//! Sensirion models (scd41) chose always-ready responses for exactly this
//! reason. Reads before the delay elapses return not-ready bytes (`0xFF`),
//! matching how a Sensirion read past an empty response buffer reads.
//!
//! **Data-ready bits.** Register devices express the same conversion timing as
//! a status bit rather than a withheld response: a
//! [`labwired_config::DataReady`] rule names the start bit firmware writes, the
//! status bit the model drives, the datasheet conversion time, and the result
//! register whose read clears the flag. It is one primitive over data — the
//! VCNL4010 adopts it in YAML, and so can any part with the same
//! start/poll/read datasheet shape. Where a bus has no honest µs source
//! (STM32-class, ESP32-classic, nRF52) the bit degrades to always-set, i.e. the
//! always-ready constant these models used before the primitive existed.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use labwired_config::{
    AddWrap, AddressRemap, AutoIncrement, Crc8Covers, Crc8Spec, DataReady, DeviceDescriptor,
    Endian, Event, FrameSpec, I2cAccess, I2cCommand, I2cRegister, I2cSpec, IndexedTable,
    ObservableSpec, ReadComplete, ResponseWord, UpdateRule,
};

use super::declarative_expr::{compile_derived, eval_derived, CompiledExpr};
use super::declarative_regs::{
    apply_timing_action, apply_write, apply_write_masked, calendar_set, civil_from_unix,
    decode_raw, decode_write, encode_raw, observe, pack, read_clears, register_read_bytes,
    unix_from_civil, unpack, validate_timers, write_is_translated, TimerBank,
};
use super::rule_machine::{RuleCtx, RuleMachine};
use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::noise::ChannelNoise;
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// CRC-8 with an arbitrary polynomial + init, no final XOR. With
/// `poly = 0x31`, `init = 0xFF` this is byte-identical to
/// [`super::sensirion::crc8`] (asserted in tests).
fn crc8(data: &[u8], poly: u8, init: u8) -> u8 {
    let mut crc = init;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ poly;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Where one [`DataReady`] rule's conversion currently stands. See the
/// lifecycle on [`DataReady`]; `Converting` carries the simulated-µs deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataReadyState {
    /// No conversion has been started (power-on), or the flag was cleared by a
    /// result read and the start bits were no longer set.
    Idle,
    /// A conversion is in flight; the flag sets once `elapsed_us` reaches this.
    Converting(u64),
    /// The conversion finished: the status bit reads set.
    Ready,
}

/// The generic device. Constructed from a [`DeviceDescriptor`] whose
/// `behavior.i2c` supplies the wire protocol.
pub struct GenericI2cDevice {
    address: u8,
    /// Register-mode registers (empty in command mode).
    registers: Vec<I2cRegister>,
    /// Command-mode commands (empty in register mode).
    commands: Vec<I2cCommand>,
    /// CRC-8 framing for command responses.
    crc8: Option<Crc8Spec>,
    command_mode: bool,
    /// Command-code width in bytes (1 or 2). A command dispatches once the
    /// master has written this many bytes.
    code_width: usize,

    /// Measurement slots keyed by input-channel key (engineering units).
    slots: HashMap<String, f64>,
    /// Current stored value per register name (rw writes + resets). Also the
    /// source a `scale_from` reads its selecting bit-field from.
    reg_values: HashMap<String, u32>,

    /// Selected register pointer for the current transaction. `u16` because a
    /// `pointer_width: 2` device addresses a 16-bit space; a 1-byte-pointer
    /// device never leaves 0x00..=0xFF (see `pointer_span`).
    pointer: Option<u16>,
    /// Bytes the master has written this transaction.
    write_buf: Vec<u8>,
    /// Bytes queued for the master to read; drained by `read`.
    read_buf: Vec<u8>,
    read_idx: usize,
    /// Register mode: whether `read_buf` has been latched for this read phase.
    latched: bool,

    /// Accumulated simulated wall-clock (µs) for `delay_us` gating.
    elapsed_us: u64,
    /// A delayed response withheld until `elapsed_us >= ready_at_us`.
    pending: Option<Vec<u8>>,
    ready_at_us: u64,
    /// True once a bus master has actually advanced this device's wall-clock —
    /// i.e. the chip has an honest absolute-µs source. Families without one
    /// (STM32, ESP32-classic, nRF52) never set it, and every [`DataReady`] bit
    /// degrades to always-set there (see [`DataReady`] for why).
    time_source_seen: bool,

    /// `data_ready` rules and their per-rule conversion state (parallel Vecs,
    /// one state per rule). Empty ⇒ every data-ready code path short-circuits,
    /// so devices that declare none are untouched.
    data_ready: Vec<DataReady>,
    dr_state: Vec<DataReadyState>,

    /// Currently selected register bank, and the pointer whose write selects it.
    /// `page_register: None` ⇒ a flat map and `page` stays 0 forever, so every
    /// device written before banks existed decodes exactly as it did.
    page: u8,
    page_register: Option<u16>,

    /// Indexed readout ports and their per-port fetch state (parallel Vecs).
    /// Empty ⇒ every indexed-table code path short-circuits.
    indexed_tables: Vec<IndexedTable>,
    it_state: Vec<DataReadyState>,

    /// Register-pointer-mode pointer mask (applied to the assembled pointer).
    /// 0xFF ⇒ no masking for a 1-byte pointer (the default; TMP102 uses 0x03).
    reg_pointer_mask: u16,
    /// Bytes the master writes to select an address, big-endian (1 or 2). See
    /// [`labwired_config::I2cSpec::pointer_width`]. Applies to BOTH the named
    /// register map and the byte-addressable register file.
    pointer_width: u8,
    /// Page size a sequential WRITE wraps within, for a memory-like part. See
    /// [`labwired_config::I2cSpec::write_page`].
    write_page: Option<u16>,
    /// Register-pointer-mode self-driving update rules (e.g. TMP102 drift).
    updates: Vec<UpdateRule>,
    /// Pointerless wire shape (`pointer_bytes: 0`): the part has ONE register
    /// at address 0 and every byte on the wire is its data. See
    /// [`I2cSpec::pointer_bytes`].
    reg_pointerless: bool,
    /// Register-pointer-mode byte-wise pointer auto-increment. False ⇒ the
    /// pointer latches one register and reads past its width return 0xFF, which
    /// is what every device written before this behaved like.
    reg_auto_increment: bool,
    /// Byte driven for an auto-increment address no register covers.
    reg_unmapped_byte: u8,

    /// **Byte-addressable register-file** mode state. `Some` ⇒ this device is a
    /// register-file device (PCA9685-style); the register/command fields above
    /// are unused. `None` ⇒ a register-pointer or command device.
    file: Option<Vec<u8>>,
    file_pointer: u16,
    file_writes_since_frame: u32,
    file_pointer_mask: u16,
    file_first_write_sets_pointer: bool,
    file_auto_increment: AutoIncrement,
    /// Engineering-unit observables derived from the register file.
    observables: Vec<ObservableSpec>,

    /// Discovery channels (leaked to `'static`; see [`DeclarativeI2cKit`]).
    channels: &'static [InputChannel],
    /// system.yaml `external_devices` id, stamped at attach.
    component_id: Option<String>,

    /// Seeded per-channel noise states, keyed by channel key, built from the
    /// `noise_sigma` / `bias` / `thermal_tau_s` input keys. Empty ⇒ the read
    /// path stays byte-identical to pre-noise behavior.
    noise: HashMap<String, ChannelNoise>,
    /// Noise-applied slot view cached for the duration of one register word in
    /// auto-increment mode, so every byte of a word carries ONE observation.
    /// `None` ⇒ resample on the next word (or on the next read phase).
    observed: Option<HashMap<String, f64>>,

    /// Free-running device timers (`behavior.timers`), advanced by
    /// `advance_time_us`. Empty ⇒ every timer code path short-circuits.
    timers: TimerBank,

    /// `behavior.derived` compiled once at load: named values computed from the
    /// stimulus channels on every read (see `declarative_expr`). Empty ⇒ the
    /// slot view is exactly what it was before derived channels existed.
    derived: Vec<CompiledExpr>,
    /// Hybrid auto-increment jumps (`I2cSpec.auto_increment_map`). Empty ⇒ the
    /// pointer always steps by one.
    auto_increment_map: Vec<AddressRemap>,
    /// `(channel key, the `config:` key that seeds it)` for every declared
    /// input — see `seed_from_config`.
    seed_keys: Vec<(String, String)>,
    /// **Tier 2**: the part's states, variables, FIFOs and output pins (see
    /// [`RuleMachine`]). `None` ⇒ the descriptor declares none of it, and every
    /// rule code path below short-circuits — which is what keeps every Tier-1
    /// device's transcript byte-identical. Note it holds no timer state of its
    /// own: `timers` above is the ONE clock, and a rule listens for the events
    /// it fires.
    rules: Option<RuleMachine>,
    /// Message framing, when the part declares any (see [`FrameSpec`]).
    frames: Option<FrameSpec>,
    /// The frame's own MOSI bytes, for `frame_byte(N)`. Same field, same
    /// contract and same reason as the SPI twin — a framed part must be able to
    /// read its whole message, not only the byte that closed it.
    frame_buf: Vec<u8>,
    /// The last byte pushed CLOSED a frame; the next one starts a new buffer.
    frame_closed: bool,
    /// Bytes the master has written since the last `frame` event, for a
    /// [`FrameSpec`] with a fixed `length`. Reset by every frame boundary.
    frame_bytes: u16,
}

impl GenericI2cDevice {
    /// Build from a descriptor and pre-leaked channel table.
    pub fn from_descriptor(
        descriptor: &DeviceDescriptor,
        address: u8,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        validate_descriptor(descriptor)?;
        let spec = descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c device is missing behavior.i2c")?;

        let address = if address == 0 {
            spec.default_address
        } else {
            address
        };

        // Seed measurement slots from the declared input defaults.
        let mut slots = HashMap::new();
        if let Some(meta) = &descriptor.metadata {
            for input in &meta.inputs {
                slots.insert(input.key.clone(), input.default.unwrap_or(0.0));
            }
        }
        // Seed every register to its reset value so a scale_from / storage read
        // (or a self-driving `add_wrap` update) before any write observes the
        // power-on state.
        let reg_values = spec
            .registers
            .iter()
            .map(|r| (r.name.clone(), r.reset))
            .collect();

        // Byte-addressable register-file mode: allocate the file (at its
        // power-on `fill` — an erased EEPROM cell reads 0xFF, not 0) and stamp
        // the sparse resets over it.
        let file = spec.register_file.as_ref().map(|rf| {
            let mut regs = vec![rf.fill.unwrap_or(0); rf.size];
            for (&off, &v) in &rf.reset {
                if let Some(slot) = regs.get_mut(off as usize) {
                    *slot = v;
                }
            }
            regs
        });
        let (file_pointer_mask, file_first_write_sets_pointer, file_auto_increment) =
            match &spec.register_file {
                Some(rf) => (
                    rf.pointer_mask,
                    rf.first_write_after_start_sets_pointer,
                    rf.auto_increment.clone(),
                ),
                None => (0xFF, true, AutoIncrement::Never),
            };

        // A 2-byte pointer masks to the whole 16-bit space by default; a 1-byte
        // pointer to 0xFF, which is what every device predating this had.
        let pointer_width = spec.pointer_width.max(1);
        let default_pointer_mask = if pointer_width >= 2 { 0xFFFF } else { 0x00FF };

        let mut device = Self {
            address,
            registers: spec.registers.clone(),
            commands: spec.commands.clone(),
            crc8: spec.crc8,
            command_mode: !spec.commands.is_empty(),
            code_width: spec.code_width as usize,
            slots,
            reg_values,
            // A pointerless part has nothing to select: register 0 is always
            // the target, from power-on and after every STOP.
            pointer: (spec.pointer_bytes == 0).then_some(0),
            write_buf: Vec::with_capacity(8),
            read_buf: Vec::new(),
            read_idx: 0,
            latched: false,
            elapsed_us: 0,
            pending: None,
            ready_at_us: 0,
            time_source_seen: false,
            dr_state: vec![DataReadyState::Idle; spec.data_ready.len()],
            data_ready: spec.data_ready.clone(),
            page: 0,
            page_register: spec.page_register,
            it_state: vec![DataReadyState::Idle; spec.indexed_tables.len()],
            indexed_tables: spec.indexed_tables.clone(),
            reg_pointer_mask: spec.pointer_mask.unwrap_or(default_pointer_mask),
            pointer_width,
            write_page: spec.write_page,
            reg_pointerless: spec.pointer_bytes == 0,
            updates: spec.updates.clone(),
            reg_auto_increment: spec.auto_increment,
            reg_unmapped_byte: spec.unmapped_byte.unwrap_or(0xFF),
            file,
            file_pointer: 0,
            file_writes_since_frame: 0,
            file_pointer_mask,
            file_first_write_sets_pointer,
            file_auto_increment,
            observables: spec.observables.clone(),
            channels,
            component_id: None,
            timers: TimerBank::new(&descriptor.behavior.timers),
            derived: compile_derived(
                &descriptor.behavior.derived,
                &descriptor
                    .metadata
                    .as_ref()
                    .map(|m| m.inputs.iter().map(|i| i.key.clone()).collect::<Vec<_>>())
                    .unwrap_or_default(),
            )?,
            auto_increment_map: spec.auto_increment_map.clone(),
            seed_keys: descriptor
                .metadata
                .as_ref()
                .map(|m| {
                    m.inputs
                        .iter()
                        .map(|i| {
                            (
                                i.key.clone(),
                                i.config_key.clone().unwrap_or_else(|| i.key.clone()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            noise: descriptor
                .metadata
                .as_ref()
                .map(|meta| {
                    meta.inputs
                        .iter()
                        .filter(|i| {
                            i.noise_sigma.is_some() || i.bias.is_some() || i.thermal_tau_s.is_some()
                        })
                        .map(|i| {
                            (
                                i.key.clone(),
                                ChannelNoise::new(
                                    0,  // run seed: 0 is still fully deterministic
                                    "", // re-keyed with the component id at attach
                                    &i.key,
                                    i.noise_sigma.unwrap_or(0.0),
                                    i.bias.unwrap_or(0.0),
                                    i.thermal_tau_s,
                                ),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            observed: None,
            rules: RuleMachine::from_behavior(&descriptor.behavior)?,
            frames: descriptor.behavior.frames.clone(),
            frame_bytes: 0,
            frame_buf: Vec::new(),
            frame_closed: false,
        };
        // Resolve any field-driven timer period against the RESET register
        // file, so a part whose rate register powers up at something other
        // than its `period_us` ticks correctly before firmware writes anything.
        // The DS3231 is exactly that: CONTROL powers up at 0x1C, whose RS bits
        // select 8.192 kHz, not the 1 Hz a constant would have assumed.
        device.refresh_field_driven_periods();
        Ok(device)
    }

    /// The slot view a read observes: seeded noise applied to the channels that
    /// declare it — one sample per channel per call, so a register word is a
    /// single observation, matching how firmware experiences a noisy sensor.
    /// Thermal lag uses the same accumulated µs source as `delay_us` gating;
    /// buses without an honest µs source get noise+bias but no lag.
    fn observed_slots(&mut self) -> HashMap<String, f64> {
        let mut view = if self.noise.is_empty() {
            self.slots.clone()
        } else {
            let now = self.time_source_seen.then_some(self.elapsed_us);
            self.slots
                .iter()
                .map(|(k, &v)| {
                    let v = match self.noise.get_mut(k) {
                        Some(n) if !n.is_noop() => n.sample(v, now),
                        _ => v,
                    };
                    (k.clone(), v)
                })
                .collect()
        };
        // Derived channels are computed from the OBSERVED values, so a derived
        // quantity is a function of what the part measured (noise, bias and
        // thermal lag included) rather than of the noiseless stimulus behind it.
        if !self.derived.is_empty() {
            eval_derived(&self.derived, &mut view);
        }
        view
    }

    /// Read a named observable channel in engineering units (e.g. the PCA9685
    /// `servo_angle` for a channel). Mirrors `IrCore::observable`; only
    /// register-file devices declare observables, so this returns `None` for
    /// register-pointer / command devices.
    /// The word a master would read out of a named register RIGHT NOW,
    /// sign-extended when the register is `signed:`.
    ///
    /// This is the readback a Rust or wasm consumer used to get by downcasting
    /// to a concrete hand-written model and calling its `sample()`. A part that
    /// becomes a descriptor has no concrete type to downcast to, and there is
    /// no reason each port should invent one: the thing those callers actually
    /// wanted is the register value, which is the same question for every
    /// declarative part.
    ///
    /// Register-pointer (`registers:`) mode only — a register-file part has
    /// [`observable`](Self::observable) instead. `None` for an undeclared name.
    pub fn register_word(&self, name: &str) -> Option<i64> {
        let reg = self.registers.iter().find(|r| r.name == name)?;
        let raw = unpack(
            &register_read_bytes(reg, &self.slots, &self.reg_values),
            reg.endian,
        );
        if !reg.signed {
            return Some(i64::from(raw));
        }
        let bits = 8 * u32::from(reg.width);
        if bits < 32 && raw & (1 << (bits - 1)) != 0 {
            return Some(i64::from(raw as i32 | !((1i32 << bits) - 1)));
        }
        Some(i64::from(raw))
    }

    pub fn observable(&self, name: &str, channel: u8) -> Option<f64> {
        let regs = self.file.as_ref()?;
        let obs = self.observables.iter().find(|o| o.name == name)?;
        observe(regs, obs, channel)
    }

    /// Live auto-increment check for the register-file pointer, reading the
    /// enable field out of the current register image `regs`.
    fn file_ai_enabled(auto_increment: &AutoIncrement, regs: &[u8]) -> bool {
        match auto_increment {
            AutoIncrement::Always => true,
            AutoIncrement::Never => false,
            AutoIncrement::WhenFieldSet { addr, mask } => {
                regs.get(*addr as usize).is_some_and(|r| r & *mask != 0)
            }
        }
    }

    /// Whether a `read_complete` trigger names the register at `pointer`.
    fn trigger_matches(&self, rc: &ReadComplete, pointer: u16) -> bool {
        if rc.pointer == Some(pointer) {
            return true;
        }
        match (&rc.register, self.find_register(pointer)) {
            (Some(name), Some(reg)) => &reg.name == name,
            _ => false,
        }
    }

    /// Apply every `add_wrap` update whose `read_complete` trigger names the
    /// just-fully-read register at `pointer`, mutating the register's stored
    /// word (signed i16 semantics, matching the reference drift model).
    fn apply_read_complete_updates(&mut self, pointer: u16) {
        let actions: Vec<AddWrap> = self
            .updates
            .iter()
            .filter(|u| self.trigger_matches(&u.trigger.read_complete, pointer))
            .map(|u| u.action.add_wrap.clone())
            .collect();
        if actions.is_empty() {
            return;
        }
        let Some(name) = self.find_register(pointer).map(|r| r.name.clone()) else {
            return;
        };
        for a in actions {
            let cur = self.reg_values.get(&name).copied().unwrap_or(0) as u16 as i16;
            let mut v = cur.wrapping_add(a.add);
            if v > a.max {
                v = a.reset;
            }
            self.reg_values.insert(name.clone(), (v as u16) as u32);
        }
    }

    // ─── data_ready primitive ──────────────────────────────────────────────
    //
    // One write-triggered, time-gated status bit, driven entirely by the
    // declared [`DataReady`] rules. Every method here returns immediately when
    // no rule is declared, so devices without the primitive are byte-identical.

    /// Promote every conversion whose deadline the simulated clock has reached.
    /// Called before a register read latches, which is the only moment the
    /// state is observable.
    fn tick_data_ready(&mut self) {
        for state in &mut self.dr_state {
            if let DataReadyState::Converting(deadline) = *state {
                if self.elapsed_us >= deadline {
                    *state = DataReadyState::Ready;
                }
            }
        }
    }

    /// Whether rule `i`'s status bit currently reads set. Without an honest µs
    /// source the bit is always set — the documented holdout degradation.
    fn data_ready_set(&self, i: usize) -> bool {
        !self.time_source_seen || self.dr_state[i] == DataReadyState::Ready
    }

    /// The bits every declared rule contributes to a read of `register` — both
    /// `data_ready` conversion flags and `indexed_tables` fetch strobes.
    fn ready_overlay(&self, register: &str) -> u32 {
        let mut overlay = 0;
        for (i, rule) in self.data_ready.iter().enumerate() {
            if rule.ready_register == register && self.data_ready_set(i) {
                overlay |= rule.ready_mask;
            }
        }
        for (i, table) in self.indexed_tables.iter().enumerate() {
            if table.strobe_register == register && self.it_state[i] == DataReadyState::Ready {
                overlay |= table.strobe_mask;
            }
        }
        overlay
    }

    // ─── indexed_table primitive ───────────────────────────────────────────
    //
    // Write an index, arm the strobe, poll the strobe, read the latched word.
    // Every method returns immediately when no port is declared.

    /// Promote every fetch whose access time the simulated clock has reached.
    /// Called from the same places `tick_data_ready` is: just before a read is
    /// observable.
    fn tick_indexed_tables(&mut self) {
        for state in &mut self.it_state {
            if let DataReadyState::Converting(deadline) = *state {
                if self.elapsed_us >= deadline {
                    *state = DataReadyState::Ready;
                }
            }
        }
    }

    /// A write of `strobe_arm_value` to a port's strobe register arms a fetch:
    /// the strobe bits drop, the word at the current index is latched into the
    /// data register, and the strobe re-raises once `access_us` has elapsed.
    /// `written` is the RAW value the master put on the wire (not the
    /// `write_mask`-filtered store), because "the master wrote 0x00" is the
    /// event silicon reacts to.
    fn arm_indexed_tables(&mut self, register: &str, written: u32) {
        for i in 0..self.indexed_tables.len() {
            let table = &self.indexed_tables[i];
            if table.strobe_register != register || written != table.strobe_arm_value {
                continue;
            }
            let index = self
                .reg_values
                .get(&table.index_register)
                .copied()
                .unwrap_or(0) as u8;
            let word = table.entries.get(&index).copied().unwrap_or(0);
            let data_register = table.data_register.clone();
            let deadline = self.elapsed_us.saturating_add(table.access_us);
            self.reg_values.insert(data_register, word);
            self.it_state[i] = DataReadyState::Converting(deadline);
        }
    }

    /// Start every conversion whose start bits the master just left set in
    /// `register` (level-triggered — a driver re-issues the same on-demand bit
    /// for each reading). `stored` is the register's value AFTER the write.
    fn start_conversions(&mut self, register: &str, stored: u32) {
        for (i, rule) in self.data_ready.iter().enumerate() {
            if rule.start_register == register && stored & rule.start_mask != 0 {
                self.dr_state[i] =
                    DataReadyState::Converting(self.elapsed_us.saturating_add(rule.conversion_us));
            }
        }
    }

    /// Clear every status bit whose result register was just read, and restart
    /// the conversion when the start bits are still set (so a periodic /
    /// self-timed sketch keeps getting fresh data instead of stalling).
    fn clear_on_read(&mut self, register: &str) {
        for i in 0..self.data_ready.len() {
            if !self.data_ready[i]
                .clear_on_read
                .iter()
                .any(|r| r == register)
            {
                continue;
            }
            let rule = &self.data_ready[i];
            let still_started = self
                .reg_values
                .get(&rule.start_register)
                .copied()
                .unwrap_or(0)
                & rule.start_mask
                != 0;
            self.dr_state[i] = if still_started {
                DataReadyState::Converting(self.elapsed_us.saturating_add(rule.conversion_us))
            } else {
                DataReadyState::Idle
            };
        }
    }

    /// Write one byte at `addr` on the byte-wise auto-increment path: merge it
    /// into the byte of the covering register it lands on, then — once the
    /// register's LAST byte has arrived — run the post-write side effects
    /// exactly once (bank select, acknowledge, conversion start, indexed-table
    /// arm, self-clearing "go" bits).
    /// The translated write: the wire word a master put on the bus turned into
    /// the word this register STORES.
    ///
    /// * `bcd:` — the nibbles are decoded to the integer the model keeps, so
    ///   every expression that reads the register (`reg()`, `field()`,
    ///   `scale_from`) is in decimal.
    /// * `calendar:` — the register is one civil field of the clock channel
    ///   named by `source:`, so the write RECOMPOSES that instant: this field
    ///   is replaced and the other six are left where they were. Without it a
    ///   `source`d register would be read-only and `RTClib::adjust()` — the
    ///   first call an RTC sketch makes — would do nothing at all.
    ///
    /// `write_mask` is applied in the WIRE domain (a plain AND on the word the
    /// master wrote) rather than as the usual keep-the-bits-outside-it merge:
    /// "keep the previous bit" has no meaning across a domain change, and what
    /// the datasheets mask here is a neighbouring flag (the DS3231 `CH` bit
    /// shares the seconds byte) that is not part of the number at all.
    fn commit_translated_write(&mut self, reg: &I2cRegister, written: u32) -> u32 {
        let wire = written & reg.write_mask.unwrap_or(u32::MAX);
        let decoded = decode_write(reg, wire);
        if let (Some(field), Some(src)) = (reg.calendar, reg.source.as_ref()) {
            let now = self.slots.get(src).copied().unwrap_or(0.0);
            let mut civil = civil_from_unix(now as i64);
            calendar_set(&mut civil, field, i64::from(decoded));
            self.slots
                .insert(src.clone(), unix_from_civil(civil) as f64);
        }
        decoded
    }

    fn write_byte_at(&mut self, addr: u16, data: u8) {
        // The bank select is answered before any register decode: it is what
        // decides which register the NEXT pointer means.
        if self.page_register == Some(addr) {
            self.page = data;
        }
        let Some(reg) = self.register_covering(addr) else {
            return;
        };
        if reg.access != I2cAccess::Rw {
            return;
        }
        let (name, endian, width, write_mask, self_clearing, on_write) = (
            reg.name.clone(),
            reg.endian,
            reg.width,
            reg.write_mask,
            reg.self_clearing,
            reg.on_write.unwrap_or(labwired_config::WriteAction::None),
        );
        // Taken before the mutation below so the borrow of the descriptor ends
        // here; `None` for the ordinary (untranslated) register, which is every
        // register that does not use `bcd:` or `calendar:`.
        let translated: Option<I2cRegister> = write_is_translated(reg).then(|| reg.clone());
        let idx = usize::from(addr - reg.addr);
        let prev = self.reg_values.get(&name).copied().unwrap_or(0);
        // Place the byte at its position in the word, honouring the declared
        // byte order, so a byte-wise burst reassembles the same word a
        // width-sized write would have stored.
        let shift = 8 * match endian {
            Endian::Be => u32::from(width) - 1 - idx as u32,
            Endian::Le => idx as u32,
        };
        // ONE byte lane arrives at a time here, so the datasheet write action
        // applies to that lane alone — a write-1-to-clear byte must not clear
        // bits in the bytes of the word the master has not written yet.
        // Narrowing the writable mask to the lane does that, and for a plain
        // store it is byte-identical to the merge this path always did.
        let lane = 0xFFu32 << shift;
        let raw = u32::from(data) << shift;
        let written = (prev & !lane) | raw;
        let stored = if let Some(reg) = translated {
            // A BCD / `calendar:` register's stored word is in a DIFFERENT
            // domain from the wire, so lane-merging it with `prev` would mix
            // nibbles into decimal. The wire word is assembled from the bytes
            // written so far and translated on the LAST byte; every register
            // that uses either key today is one byte wide, where there is no
            // intermediate state at all.
            let wire = (prev & !lane) | raw;
            if idx + 1 == usize::from(width) {
                self.commit_translated_write(&reg, wire)
            } else {
                wire
            }
        } else {
            apply_write_masked(on_write, prev, raw, write_mask.unwrap_or(u32::MAX) & lane)
        };
        self.reg_values.insert(name.clone(), stored);
        if idx + 1 != usize::from(width) {
            return; // mid-word: side effects fire once, on the last byte
        }
        if !self.data_ready.is_empty() {
            // Acknowledge first, then start — see the non-incrementing path.
            self.clear_on_write(&name);
            self.start_conversions(&name, stored);
        }
        if !self.indexed_tables.is_empty() {
            self.arm_indexed_tables(&name, written);
        }
        if !self.timers.is_empty() {
            self.timers.start_on_write(&name, stored, self.elapsed_us);
            self.refresh_field_driven_periods();
        }
        // A momentary "go" bit is gone by the time firmware can read it back:
        // the device has already acted on it (see `RegisterSpec::self_clearing`).
        if let Some(mask) = self_clearing {
            if stored & mask != 0 {
                self.reg_values.insert(name.clone(), stored & !mask);
            }
        }
        // Tier 2 LAST, so a rule sees the post-side-effect register.
        self.raise_and_settle(
            Event::Write {
                register: name,
                field: None,
            },
            i64::from(written),
        );
    }

    /// Store one byte into the single register of a pointerless part, then run
    /// the same post-write side effects a pointered write runs.
    fn write_pointerless(&mut self, data: u8) {
        let Some(reg) = self.find_register(0) else {
            return;
        };
        if reg.access != I2cAccess::Rw {
            return;
        }
        let (name, write_mask) = (reg.name.clone(), reg.write_mask);
        let written = u32::from(data);
        let stored = match write_mask {
            Some(mask) => {
                let prev = self.reg_values.get(&name).copied().unwrap_or(0);
                (prev & !mask) | (written & mask)
            }
            None => written,
        };
        self.reg_values.insert(name.clone(), stored);
        if !self.data_ready.is_empty() {
            self.clear_on_write(&name);
            self.start_conversions(&name, stored);
        }
        self.raise_and_settle(
            Event::Write {
                register: name,
                field: None,
            },
            i64::from(written),
        );
    }

    /// Convenience for tests / standalone use: parse a descriptor YAML and leak
    /// its channel table. (The kit path shares one leaked table across attaches;
    /// this leaks per call, which is fine for the few devices a test builds.)
    pub fn from_yaml(yaml: &str, address: u8) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        let channels = leak_channels(&descriptor);
        Self::from_descriptor(&descriptor, address, channels)
    }

    /// Override one channel's Gaussian noise sigma from a `config:` value (see
    /// [`labwired_config::InputSpec::noise_sigma_key`]). The channel's declared
    /// `bias` and `thermal_tau_s` are kept; a sigma of 0 removes the noise
    /// state entirely, so a placement that sets the key to 0 is byte-identical
    /// to one that never mentioned it.
    pub fn set_channel_noise_sigma(&mut self, key: &str, sigma: f64) {
        let id = self.component_id.clone().unwrap_or_default();
        match self.noise.get(key) {
            Some(n) => {
                let (bias, tau) = (n.bias(), n.tau_s());
                if sigma <= 0.0 && bias == 0.0 && tau.is_none() {
                    self.noise.remove(key);
                } else {
                    self.noise.insert(
                        key.to_string(),
                        ChannelNoise::new(0, &id, key, sigma, bias, tau),
                    );
                }
            }
            None if sigma > 0.0 => {
                self.noise.insert(
                    key.to_string(),
                    ChannelNoise::new(0, &id, key, sigma, 0.0, None),
                );
            }
            None => {}
        }
    }

    /// Seed a measurement slot's initial value from a `config:` override. Only
    /// keys that name a declared input channel take effect (others are ignored),
    /// so a descriptor's `config_keys` like `lux` seed the part's starting
    /// reading exactly as a hand-written kit's `config_f64("lux")` did.
    pub fn seed_input(&mut self, key: &str, value: f64) {
        if self.channels.iter().any(|c| c.key == key) {
            self.slots.insert(key.to_string(), value);
        }
    }

    /// Seed every declared input channel from an `external_devices` `config:`
    /// block, given a lookup for one key.
    ///
    /// A channel may name a DIFFERENT config key than its runtime key
    /// ([`labwired_config::InputSpec::config_key`]) — the MLX90614 kit this
    /// replaces took `surface_temp_c` in `config:` and served `surface_temp` as
    /// the runtime channel, and three shipped `system.yaml` files set the
    /// former. The channel key itself is still accepted, so a file written
    /// against either spelling works.
    ///
    /// Lives here rather than in the kit because BOTH attach paths need it: the
    /// `PeripheralKit` pass and `i2c_factory::build_i2c_device`, which is what a
    /// controller that only builds slaves (the ESP32-C3 I²C, nRF TWIM) calls.
    /// Seeding in one and not the other is how the same YAML would boot at two
    /// different temperatures depending on which MCU it hung off.
    pub fn seed_from_config(&mut self, get: impl Fn(&str) -> Option<f64>) {
        for (channel, config_key) in self.seed_keys.clone() {
            if let Some(v) = get(&config_key).or_else(|| get(&channel)) {
                self.seed_input(&channel, v);
            }
        }
    }

    /// The pointer one address on, wrapped within the width the part's pointer
    /// actually has: a 1-byte pointer rolls 0xFF → 0x00 exactly as it did when
    /// the pointer was a `u8`, and a 2-byte pointer rolls at 0xFFFF.
    fn next_pointer(&self, ptr: u16) -> u16 {
        // A STREAM PORT holds the pointer (see `RegisterSpec::stream`). Checked
        // first and in this one place, so a read and a write cannot disagree
        // about whether the cursor moved.
        if self.find_register(ptr).is_some_and(|r| r.stream) {
            return ptr;
        }
        // Hybrid auto-increment: the pointer JUMPS rather than steps. Checked
        // before the step, and only here — an explicit pointer write never
        // passes through this function, which is what "only the auto-increment
        // path is remapped" means. See `I2cSpec::auto_increment_map`.
        if let Some(remap) = self.auto_increment_map.iter().find(|m| m.from == ptr) {
            return remap.to;
        }
        let span: u16 = if self.pointer_width >= 2 {
            0xFFFF
        } else {
            0x00FF
        };
        ptr.wrapping_add(1) & span
    }

    /// The write-pointer one byte on in a register FILE, honouring
    /// [`labwired_config::I2cSpec::write_page`]: a sequential write that runs
    /// off the end of a page wraps to the start of the SAME page rather than
    /// spilling into the next one. This is the real EEPROM behaviour a driver
    /// that writes a record across a page boundary trips over, and a model that
    /// only incremented would hide it. A page size that is not a power of two
    /// is handled by the modulo, so the field is not silently restricted.
    fn next_write_pointer(ptr: u16, write_page: Option<u16>) -> u16 {
        match write_page.filter(|p| *p > 1) {
            Some(page) => {
                let offset = ptr % page;
                (ptr - offset).wrapping_add((offset + 1) % page)
            }
            None => ptr.wrapping_add(1),
        }
    }

    /// The register at `addr` in the current bank. A bank-specific register wins
    /// over a bank-agnostic one at the same pointer, so a part can carry a flat
    /// core map plus a handful of aliased addresses.
    fn find_register(&self, addr: u16) -> Option<&I2cRegister> {
        self.registers
            .iter()
            .find(|r| r.addr == addr && r.page == Some(self.page))
            .or_else(|| {
                self.registers
                    .iter()
                    .find(|r| r.addr == addr && r.page.is_none())
            })
    }

    /// The register whose byte span COVERS `addr`, not just the one that starts
    /// there. Only auto-increment needs this: without it, walking into the
    /// second byte of a 2-byte register would look unmapped.
    fn register_covering(&self, addr: u16) -> Option<&I2cRegister> {
        let covers =
            |r: &&I2cRegister| addr >= r.addr && addr < r.addr.saturating_add(u16::from(r.width));
        self.registers
            .iter()
            .find(|r| covers(r) && r.page == Some(self.page))
            .or_else(|| {
                self.registers
                    .iter()
                    .find(|r| covers(r) && r.page.is_none())
            })
    }

    /// One byte of the address space, as auto-increment reads it: the byte the
    /// covering register drives (status overlay included), or `unmapped_byte`.
    ///
    /// When `addr` is the register's LAST byte, also returns its name and START
    /// address — the two things a post-word side effect needs. A mid-word byte
    /// reports `None` so clears and updates fire once per word, not per byte.
    fn byte_at(&self, addr: u16, slots: &HashMap<String, f64>) -> (u8, Option<(String, u16)>) {
        let Some(reg) = self.register_covering(addr) else {
            return (self.reg_unmapped_byte, None);
        };
        // A `fifo:` register serves the queue's OLDEST entry while the queue is
        // non-empty and falls through to its live `source:` when it is empty —
        // which is exactly what bypass mode is, with no second mode flag to
        // keep in step. Both read paths need it: this is the auto-increment
        // one (a burst that walks addresses), and the latch path below is the
        // pointer one.
        let raw = match self.fifo_word(reg) {
            Some(word) => pack(word as u32, reg.width, reg.endian),
            None => register_read_bytes(reg, slots, &self.reg_values),
        };
        let overlay = self.ready_overlay(&reg.name);
        let bytes = if overlay == 0 {
            raw
        } else {
            pack(unpack(&raw, reg.endian) | overlay, reg.width, reg.endian)
        };
        let idx = usize::from(addr - reg.addr);
        let byte = bytes.get(idx).copied().unwrap_or(self.reg_unmapped_byte);
        let done = idx + 1 == usize::from(reg.width);
        (byte, done.then(|| (reg.name.clone(), reg.addr)))
    }

    /// Write-1-to-clear: any write to a named register drops the ready bit.
    fn clear_on_write(&mut self, register: &str) {
        for i in 0..self.data_ready.len() {
            if !self.data_ready[i]
                .clear_on_write
                .iter()
                .any(|n| n == register)
            {
                continue;
            }
            if matches!(self.dr_state[i], DataReadyState::Ready) {
                self.dr_state[i] = DataReadyState::Idle;
            }
        }
    }

    fn find_command(&self, code: u16) -> Option<&I2cCommand> {
        self.commands.iter().find(|c| c.code == code)
    }

    /// Build the response bytes for a dispatched command (before delay gating).
    /// `slots` is the noise-applied observation view computed by the caller;
    /// `code` is the command the master wrote, which the SMBus PEC covers.
    fn build_response(&self, cmd: &I2cCommand, code: u16, slots: &HashMap<String, f64>) -> Vec<u8> {
        let transaction_pec = self
            .crc8
            .is_some_and(|c| c.covers == Crc8Covers::Transaction);
        let mut out = Vec::new();
        for word in &cmd.response {
            let raw = Self::response_word_raw(word, slots);
            let bytes = pack(raw, word.width, word.endian);
            match &self.crc8 {
                // CRC framing is per 16-bit word, exactly like the Sensirion
                // read buffer (see super::sensirion::encode_words).
                Some(c) if !transaction_pec => {
                    for chunk in bytes.chunks(2) {
                        out.extend_from_slice(chunk);
                        out.push(crc8(chunk, c.poly, c.init));
                    }
                }
                _ => out.extend_from_slice(&bytes),
            }
        }
        // SMBus Packet Error Code: ONE byte at the end of the frame, computed
        // over the bytes the MASTER drove as well as the ones the slave
        // answered — `[addr·W, command, addr·R, data…]` (SMBus 3.1 §6.4.1,
        // MLX90614 §8.4.3). The address is the one this device is attached at,
        // so a part moved by `i2c_address:` still answers a PEC its driver
        // accepts. A `code_width: 1` opcode contributes its single byte, which
        // is the only form SMBus defines.
        if transaction_pec {
            let c = self.crc8.expect("transaction_pec implies a crc8 spec");
            let mut frame = Vec::with_capacity(out.len() + 4);
            frame.push(self.address << 1);
            if self.code_width >= 2 {
                frame.push((code >> 8) as u8);
            }
            frame.push((code & 0xFF) as u8);
            frame.push((self.address << 1) | 1);
            frame.extend_from_slice(&out);
            out.push(crc8(&frame, c.poly, c.init));
        }
        out
    }

    fn response_word_raw(word: &ResponseWord, slots: &HashMap<String, f64>) -> u32 {
        if let Some(src) = &word.source {
            let value = slots.get(src).copied().unwrap_or(0.0);
            encode_raw(value, word.encode.as_ref(), 1.0, word.width, false)
        } else {
            word.const_value.unwrap_or(0)
        }
    }

    fn dispatch_command(&mut self, code: u16) {
        self.read_buf.clear();
        self.read_idx = 0;
        self.pending = None;
        let Some(cmd) = self.find_command(code) else {
            // Unknown command: no response queued (reads return 0xFF), matching
            // the Sensirion reference (scd41).
            return;
        };
        let cmd = cmd.clone();
        // One observation per dispatched command: the whole response frame
        // (every word + CRC) is computed from a single noise-applied slot view.
        let slots = self.observed_slots();
        let resp = self.build_response(&cmd, code, &slots);
        match cmd.delay_us {
            Some(us) if us > 0 => {
                self.pending = Some(resp);
                self.ready_at_us = self.elapsed_us + us;
            }
            _ => self.read_buf = resp,
        }
    }
}

// ─── Tier 2: the rule machine's view of this device ────────────────────────

/// The [`RuleCtx`] a declarative I²C device hands its [`RuleMachine`].
///
/// It borrows the register file mutably and the register MAP and measurement
/// slots immutably, which is exactly the split a rule needs: a rule changes
/// stored words, and reads the map (for `bits:` names and `encode:`) without
/// being able to change it.
struct I2cRuleCtx<'a> {
    registers: &'a [I2cRegister],
    reg_values: &'a mut HashMap<String, u32>,
    /// `&mut` because [`RuleCtx::set_input`] writes here — see that method.
    slots: &'a mut HashMap<String, f64>,
}

impl RuleCtx for I2cRuleCtx<'_> {
    fn reg(&self, name: &str) -> Option<u32> {
        // Declared-but-never-written registers still answer: they were seeded
        // to their reset value at construction.
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

    fn reported(&self, name: &str) -> Option<i64> {
        let reg = self.registers.iter().find(|r| r.name == name)?;
        // The SAME function the read path uses, so a rule and the wire cannot
        // disagree about what a register says.
        //
        // The TRUTH slots, not the noisy ones: a rule reading a register twice
        // in one event must see one value, and a seeded sample belongs to a
        // wire read rather than to the part's internal arithmetic.
        let bytes = register_read_bytes(reg, self.slots, self.reg_values);
        let word = unpack(&bytes, reg.endian);
        if reg.signed {
            let bits = 8 * u32::from(reg.width);
            if bits < 32 && word & (1 << (bits - 1)) != 0 {
                return Some(i64::from(word as i32 | !((1i32 << bits) - 1)));
            }
        }
        Some(i64::from(word))
    }

    fn input(&self, key: &str) -> i64 {
        let raw = self.slots.get(key).copied().unwrap_or(0.0);
        // `input(KEY)` is the value as the REGISTER would report it, so a rule
        // comparing against a register word compares like with like. When no
        // register sources the key there is no declared encoding and the honest
        // answer is the truncated engineering value.
        //
        // A `calendar:` register is NOT such an encoding: it reports one civil
        // FIELD of the instant, not the instant, so there is nothing to compare
        // like with like against. Borrowing its encode here would hand a rule
        // the BCD seconds of a clock where it asked for the Unix time — off by
        // eight orders of magnitude, and silently, because `0x99` is a
        // perfectly ordinary integer. Skipped, so such a channel falls to the
        // engineering value.
        match self
            .registers
            .iter()
            .find(|r| r.source.as_deref() == Some(key) && r.calendar.is_none())
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
                    // Sign-extend out of the register's width so arithmetic in a
                    // rule sees -1, not 0xFFFF.
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

    fn set_input(&mut self, key: &str, value: i64) {
        // The exact inverse of `input` above, through the SAME register lookup
        // (a `calendar:` register skipped for the same reason), so a rule that
        // writes back what it read changes nothing.
        if !self.slots.contains_key(key) {
            return;
        }
        let engineering = match self
            .registers
            .iter()
            .find(|r| r.source.as_deref() == Some(key) && r.calendar.is_none())
        {
            Some(reg) => decode_raw(
                value,
                reg.encode.as_ref(),
                reg.source_scale.unwrap_or(1.0),
                reg.width,
            ),
            None => value as f64,
        };
        self.slots.insert(key.to_string(), engineering);
    }
}

impl GenericI2cDevice {
    /// Raise a Tier-2 event at the rule machine. No-op — and no allocation —
    /// for a descriptor that declares no rules.
    ///
    /// The machine is moved out for the call so it can hold `&mut` on the
    /// register file while running actions. Nothing between the take and the
    /// put-back can observe the device, because `raise` takes `&mut self`.
    fn raise(&mut self, event: Event, written: i64) {
        let Some(mut machine) = self.rules.take() else {
            return;
        };
        {
            let mut ctx = I2cRuleCtx {
                registers: &self.registers,
                reg_values: &mut self.reg_values,
                slots: &mut self.slots,
            };
            machine.fire(&event, written, &mut ctx);
        }
        self.rules = Some(machine);
    }

    /// Fire a Tier-2 event and immediately apply any `timer:` action it queued.
    ///
    /// Separate from [`raise`](Self::raise) because the timer drive calls
    /// `raise` itself and must not re-enter the drain mid-walk; every other
    /// entry point goes through this one, so a rule that starts a conversion
    /// timer has it armed before the next bus byte.
    fn raise_and_settle(&mut self, event: Event, written: i64) {
        self.raise(event, written);
        self.drain_timer_requests();
    }

    /// Close the frame the wire just completed: hand the machine the frame's
    /// BYTES, then raise the event. The SPI twin carries the argument for the
    /// ordering; it is the same one, on the other transport.
    fn close_frame(&mut self, written: i64) {
        let opcode = self.frames.as_ref().is_some_and(|f| f.opcode_byte);
        if self.rules.is_some() {
            let buf = std::mem::take(&mut self.frame_buf);
            if let Some(m) = self.rules.as_mut() {
                m.set_frame_bytes(&buf, opcode);
            }
            self.frame_buf = buf;
        }
        self.frame_closed = true;
        self.raise_and_settle(Event::Frame, written);
    }

    /// Let the rule machine record the elapsed µs. It schedules nothing: the
    /// device's [`TimerBank`] is the one clock and raises `Event::Timer`.
    fn advance_rule_time(&mut self, us: u64) {
        if let Some(m) = self.rules.as_mut() {
            m.advance_time_us(us);
        }
    }

    /// Apply whatever `timer:` actions the rules queued to the ONE bank.
    /// Push one entry into every FIFO whose `fill.timer` is `name`, then
    /// reflect the new depth into the part's `count:` and `watermark:`
    /// registers.
    fn fill_fifos_on_timer(&mut self, name: &str) {
        let Some(mut machine) = self.rules.take() else {
            return;
        };
        {
            let mut ctx = I2cRuleCtx {
                registers: &self.registers,
                reg_values: &mut self.reg_values,
                slots: &mut self.slots,
            };
            if machine.fill_on_timer(name, &mut ctx) {
                machine.refresh_fifo_registers(&mut ctx);
            }
        }
        self.rules = Some(machine);
    }

    /// The FIFO component this register serves, if it has one and the queue is
    /// non-empty. `None` ⇒ the register serves its live `source:`, which is
    /// what bypass mode is.
    fn fifo_word(&self, reg: &I2cRegister) -> Option<i64> {
        let spec = reg.fifo.as_ref()?;
        self.rules.as_ref()?.fifo_peek(&spec.name, spec.slot)
    }

    /// Pop the entry a completed read of `reg` drains, and reflect the new
    /// depth. No-op for a register with no `fifo:`, or with `pop: false`.
    fn fifo_pop_after_read(&mut self, register: &str) {
        let Some(spec) = self
            .registers
            .iter()
            .find(|r| r.name == register)
            .and_then(|r| r.fifo.clone())
            .filter(|f| f.pop)
        else {
            return;
        };
        let Some(mut machine) = self.rules.take() else {
            return;
        };
        {
            let mut ctx = I2cRuleCtx {
                registers: &self.registers,
                reg_values: &mut self.reg_values,
                slots: &mut self.slots,
            };
            if machine.fifo_pop(&spec.name) {
                machine.refresh_fifo_registers(&mut ctx);
            }
        }
        self.rules = Some(machine);
    }

    /// Re-resolve every [`TimerPeriodFrom`](labwired_config::TimerPeriodFrom)
    /// against the register file. Called wherever a register write lands, and
    /// once after construction, because a rate register is exactly the thing
    /// firmware writes.
    ///
    /// Short-circuits on a part that declares no field-driven period, which is
    /// every descriptor written before the key existed — such a part pays one
    /// `any()` over its timer list and nothing else.
    fn refresh_field_driven_periods(&mut self) {
        if !self.timers.has_field_driven_period() {
            return;
        }
        let values = std::mem::take(&mut self.reg_values);
        let specs = self.registers.clone();
        let now = self.elapsed_us;
        self.timers.apply_period_from(
            now,
            &|name: &str| values.get(name).copied(),
            &|register: &str, field: &str| {
                specs.iter().find(|r| r.name == register).and_then(|r| {
                    r.bits
                        .iter()
                        .find(|b| b.name == field)
                        .map(|b| (b.shift, b.mask()))
                })
            },
        );
        self.reg_values = values;
    }

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

    /// A timer's EFFECTIVE period in µs, after any
    /// [`TimerPeriodFrom`](labwired_config::TimerPeriodFrom) has resolved
    /// against the register file.
    ///
    /// For tests and diagnostics: a field-driven rate is otherwise only
    /// observable by counting firings, and counting firings cannot tell a
    /// RESET value apart from a coincidence.
    pub fn timer_period_us(&self, name: &str) -> Option<u64> {
        self.timers.period_us_of(name)
    }

    /// Read-only view of the rule machine, for tests and diagnostics.
    pub fn rule_machine(&self) -> Option<&RuleMachine> {
        self.rules.as_ref()
    }

    /// The wire write itself, split out so the framing counter above can store
    /// the byte and then raise `frame` without holding a borrow.
    fn write_inner(&mut self, data: u8) {
        // Register-file mode (byte-addressable): first post-START byte selects
        // the pointer; subsequent bytes are data, and the pointer auto-increments
        // when its enable field is set. The enable is checked LIVE (after the
        // store) so the write that sets the field also advances the pointer.
        if let Some(regs) = self.file.as_mut() {
            let pointer_bytes = u32::from(self.pointer_width);
            if self.file_first_write_sets_pointer && self.file_writes_since_frame < pointer_bytes {
                // Address bytes arrive HIGH byte first — the order every
                // 16-bit-addressed I²C part uses, and a no-op for a 1-byte
                // pointer, which still lands `data & mask` exactly as before.
                let acc = if self.file_writes_since_frame == 0 {
                    u16::from(data)
                } else {
                    (self.file_pointer << 8) | u16::from(data)
                };
                self.file_pointer = acc & self.file_pointer_mask;
            } else if !regs.is_empty() {
                let idx = self.file_pointer as usize % regs.len();
                regs[idx] = data;
                if Self::file_ai_enabled(&self.file_auto_increment, regs) {
                    self.file_pointer =
                        Self::next_write_pointer(self.file_pointer, self.write_page);
                }
            }
            self.file_writes_since_frame = self.file_writes_since_frame.saturating_add(1);
            return;
        }
        self.write_buf.push(data);
        if self.command_mode {
            // A command completes once `code_width` bytes have arrived (a
            // 16-bit big-endian Sensirion opcode, or a single-byte BH1750-style
            // opcode). Parameter words follow but are accepted and ignored
            // (params_words); write_buf keeps growing so this never re-fires.
            if self.write_buf.len() == self.code_width {
                let code = self.write_buf[..self.code_width]
                    .iter()
                    .fold(0u16, |acc, &b| (acc << 8) | b as u16);
                self.dispatch_command(code);
            }
            return;
        }
        // Pointerless (PCF8574-shaped): there is no address byte at all. Every
        // byte is data for the single register at 0, and a multi-byte write is
        // a sequence of updates to it — which is exactly what an expander does
        // when firmware streams port values without re-addressing. This is the
        // `pointer_width: 0` end of the same axis `pointer_width: 2` is at.
        if self.reg_pointerless {
            let word = *self.write_buf.last().unwrap_or(&data);
            self.write_buf.clear();
            self.write_pointerless(word);
            return;
        }
        // Register mode: the first `pointer_width` bytes are the pointer
        // (big-endian, masked); the rest are a data write into the pointed rw
        // register. With the default width of 1 this is byte-for-byte the
        // single-pointer-byte path every device has always taken.
        let pointer_bytes = usize::from(self.pointer_width);
        if self.write_buf.len() <= pointer_bytes {
            if self.write_buf.len() == pointer_bytes {
                let acc = self.write_buf[..pointer_bytes]
                    .iter()
                    .fold(0u16, |a, &b| (a << 8) | u16::from(b));
                self.pointer = Some(acc & self.reg_pointer_mask);
            }
            return;
        }
        let Some(ptr) = self.pointer else { return };
        // Byte-wise auto-increment applies to WRITES as well as reads: the
        // pointer IS the cursor, so a block write streams consecutive registers
        // in one transaction. ST's VL53L0X API writes the 6-byte reference-SPAD
        // enable map that way (`VL53L0X_WriteMulti` at 0xB0) and then reads it
        // back and compares, so a model that only accepted a write whose length
        // exactly matched one register's width would fail that comparison.
        if self.reg_auto_increment {
            self.pointer = Some(self.next_pointer(ptr));
            self.write_byte_at(ptr, data);
            return;
        }
        let Some(reg) = self.find_register(ptr) else {
            return;
        };
        if reg.access != I2cAccess::Rw || self.write_buf.len() != pointer_bytes + reg.width as usize
        {
            return;
        }
        let (name, endian) = (reg.name.clone(), reg.endian);
        let reg = reg.clone();
        let written = unpack(&self.write_buf[pointer_bytes..], endian);
        // `write_mask` protects the bits silicon owns (a model-driven status
        // flag, a hardwired bit): those keep their current value. Absent ⇒ the
        // whole word is replaced, exactly as before the mask existed. What the
        // masked bits then DO is `on_write` — a plain store unless the
        // datasheet says write-1-to-clear / zero-to-clear / one-to-set.
        let prev = self.reg_values.get(&name).copied().unwrap_or(0);
        let stored = if write_is_translated(&reg) {
            self.commit_translated_write(&reg, written)
        } else {
            apply_write(&reg, prev, written)
        };
        self.reg_values.insert(name.clone(), stored);
        if !self.timers.is_empty() {
            self.timers.start_on_write(&name, stored, self.elapsed_us);
            self.refresh_field_driven_periods();
        }
        if !self.data_ready.is_empty() {
            // Acknowledge first, then start: a part whose start and clear
            // registers are the same would otherwise clear the conversion it
            // just started. Ordering it this way makes "write 1 to clear, then
            // write 1 to start" behave like the two operations it is.
            self.clear_on_write(&name);
            self.start_conversions(&name, stored);
        }
        // Tier 2 LAST, so a rule sees the post-side-effect register.
        self.raise_and_settle(
            Event::Write {
                register: name,
                field: None,
            },
            i64::from(written),
        );
    }
}

impl I2cDevice for GenericI2cDevice {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // (Re)START frames a new phase within the transaction: rewind the read
        // cursor and clear the register latch and the write accumulator. The
        // pointer (register mode) and any pending delayed response survive.
        self.write_buf.clear();
        self.read_idx = 0;
        self.latched = false;
        // A new read phase is a new observation in auto-increment mode.
        self.observed = None;
        // Register-file mode: the first write after START selects the pointer,
        // exactly like the hand-written PCA9685 (which resets its write counter
        // on START only).
        self.file_writes_since_frame = 0;
        if self.reg_pointerless {
            self.pointer = Some(0);
        }
        self.raise_and_settle(Event::Start, 0);
    }

    fn stop(&mut self) {
        // End of transaction: clear the write accumulator so the next command /
        // pointer starts fresh (the C3 controller only calls start() on a
        // repeated START, so the real reset happens here — same as veml7700 /
        // scd41).
        self.write_buf.clear();
        self.raise_and_settle(Event::Stop, 0);
        // A transaction boundary always closes a frame, so a SHORT message is
        // delivered rather than silently swallowed (see `FrameSpec`) — the
        // shape a command shell needs, where a truncated command must be seen
        // and rejected rather than waited on forever.
        if self.frames.is_some() {
            self.frame_bytes = 0;
            // A frame the LENGTH already closed leaves nothing new on the wire,
            // so this boundary frame carries no bytes rather than re-serving a
            // message the rules have already handled.
            if self.frame_closed {
                self.frame_buf.clear();
                self.frame_closed = false;
            }
            // Same contract as the SPI twin: see `FrameSpec::discard_partial`.
            if self.frames.as_ref().is_some_and(|f| f.discard_partial) {
                self.frame_buf.clear();
            } else {
                self.close_frame(0);
            }
        }
    }

    fn write(&mut self, data: u8) {
        // Framing, if the part declares any: count the bytes the master put on
        // the wire and close the frame the moment the declared length is
        // reached, WITHOUT waiting for a STOP. A fixed-length command shell is
        // expected to act on the last byte of the command, not on the end of
        // the transaction — a master that streams two commands in one
        // transaction must get two frames.
        if self.frames.is_some() {
            if self.frame_closed {
                self.frame_buf.clear();
                self.frame_closed = false;
            }
            self.frame_buf.push(data);
        }
        if let Some(length) = self.frames.as_ref().and_then(|f| f.length) {
            if length > 0 {
                self.frame_bytes = self.frame_bytes.saturating_add(1);
                if self.frame_bytes >= length {
                    self.frame_bytes = 0;
                    // The byte itself is stored below FIRST; the frame event is
                    // raised after, so a rule sees the complete message. The
                    // borrow is released by the time `raise` runs.
                    self.write_inner(data);
                    self.close_frame(i64::from(data));
                    return;
                }
            }
        }
        self.write_inner(data);
    }

    fn read(&mut self) -> u8 {
        // Register-file mode: return the pointed byte and auto-increment (live).
        if let Some(regs) = self.file.as_ref() {
            if regs.is_empty() {
                return 0;
            }
            let idx = self.file_pointer as usize % regs.len();
            let v = regs[idx];
            if Self::file_ai_enabled(&self.file_auto_increment, regs) {
                // Sequential READ rolls over the whole array — `write_page`
                // wraps writes only, which is what the EEPROM datasheets say.
                self.file_pointer = self.file_pointer.wrapping_add(1);
            }
            return v;
        }
        if self.command_mode {
            if self.pending.is_some() && self.elapsed_us >= self.ready_at_us {
                self.read_buf = self.pending.take().unwrap();
                self.read_idx = 0;
            }
            let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
            self.read_idx += 1;
            return byte;
        }
        // Register mode with byte-wise auto-increment: every read drives the
        // byte at the pointer and walks it, so a master can pull a contiguous
        // block in one transaction. No latch — the pointer IS the cursor.
        if self.reg_auto_increment {
            if !self.data_ready.is_empty() {
                self.tick_data_ready();
            }
            if !self.indexed_tables.is_empty() {
                self.tick_indexed_tables();
            }
            let addr = self.pointer.unwrap_or(0);
            // One observation per register word: the noise-applied slot view is
            // sampled when a word starts and held until its last byte is out.
            if self.observed.is_none() {
                self.observed = Some(self.observed_slots());
            }
            let (byte, hit) = match self.observed.as_ref() {
                Some(observed) => self.byte_at(addr, observed),
                None => unreachable!("observed was just populated"),
            };
            self.pointer = Some(self.next_pointer(addr));
            // Clear only once the whole word has been delivered: clearing on the
            // first byte of a 2-byte result would drop the flag while the master
            // is still mid-read. Same reason the self-driving updates fire here
            // and are keyed on the register's START address, which is what
            // `apply_read_complete_updates` matches a trigger against.
            if let Some((name, start)) = hit {
                // The FIFO entry pops when the LAST byte of the register
                // carrying `pop: true` has been clocked out. A driver that
                // abandons the burst earlier gets the same sample again, which
                // is what the silicon does with a read that never completed.
                self.fifo_pop_after_read(&name);
                if !self.data_ready.is_empty() {
                    self.clear_on_read(&name);
                }
                // `on_read: clear` — the word has been delivered in full, so
                // the read has COMPLETED (see `RegisterSpec::on_read`).
                if self.find_register(start).is_some_and(read_clears) {
                    self.reg_values.insert(name.clone(), 0);
                }
                if !self.updates.is_empty() {
                    self.apply_read_complete_updates(start);
                }
                // Word complete: the next word is a new observation.
                self.observed = None;
                // Tier 2: the read event fires once the WHOLE word is out, for
                // the same reason `clear_on_read` does — a rule that drops an
                // interrupt line must not drop it mid-word.
                self.raise_and_settle(Event::Read { register: name }, 0);
            }
            return byte;
        }
        // Register mode: latch the pointed register's bytes on the first read.
        if !self.latched {
            // Any conversion whose deadline has passed becomes readable here —
            // the only point at which the status bit is observable.
            if !self.data_ready.is_empty() {
                self.tick_data_ready();
            }
            if !self.indexed_tables.is_empty() {
                self.tick_indexed_tables();
            }
            let slots = self.observed_slots();
            let (bytes, name) = match self.pointer.and_then(|p| self.find_register(p)) {
                Some(reg) => {
                    // A `fifo:` register serves the queue's OLDEST entry while
                    // the queue is non-empty, and falls through to its live
                    // `source:` when it is empty — which is exactly what
                    // bypass mode is, with no second mode flag to keep in step.
                    let raw = match self.fifo_word(reg) {
                        Some(word) => pack(word as u32, reg.width, reg.endian),
                        None => register_read_bytes(reg, &slots, &self.reg_values),
                    };
                    // Status bits are OR'd over whatever the register stores, so
                    // one register carries the firmware-written enable bits and
                    // the model-driven ready flags at once.
                    let overlay = self.ready_overlay(&reg.name);
                    let bytes = if overlay == 0 {
                        raw
                    } else {
                        pack(unpack(&raw, reg.endian) | overlay, reg.width, reg.endian)
                    };
                    (bytes, Some(reg.name.clone()))
                }
                // Unknown pointer reads a zero word, matching veml7700.
                None => (vec![0, 0], None),
            };
            let clears_on_read = self
                .pointer
                .and_then(|p| self.find_register(p))
                .is_some_and(read_clears);
            self.read_buf = bytes;
            // The datasheets clear the flag on a read of the result register
            // ("reset when one of the corresponding result registers is read"),
            // so it happens as the read latches, not after the last byte.
            if let Some(name) = name {
                if !self.data_ready.is_empty() {
                    self.clear_on_read(&name);
                }
                // `on_read: clear` acts at the SAME moment, for the same
                // datasheet reason ("reset when the corresponding result
                // register is read") — the master keeps the bytes already
                // latched above. See `RegisterSpec::on_read`.
                if clears_on_read {
                    self.reg_values.insert(name.clone(), 0);
                }
                // Tier 2 LAST, so a rule sees the register AFTER both built-in
                // clears — the post-side-effect contract the rule machine
                // documents.
                self.raise_and_settle(Event::Read { register: name }, 0);
            }
            self.latched = true;
        }
        let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
        self.read_idx += 1;
        // Pointerless part: there is nothing to walk past. The datasheet's
        // "reading from the port" says every byte the master clocks is a fresh
        // read of the port, so re-arm the latch instead of running off the end
        // of a one-byte register and returning open-bus 0xFF.
        if self.reg_pointerless {
            self.latched = false;
            self.read_idx = 0;
        }
        // Self-driving updates: fire when the full multi-byte word has just been
        // consumed (e.g. the TMP102 +0.5 °C drift after each temperature read).
        if !self.updates.is_empty() {
            if let Some(ptr) = self.pointer {
                if let Some(width) = self.find_register(ptr).map(|r| r.width as usize) {
                    if self.read_idx == width {
                        self.apply_read_complete_updates(ptr);
                    }
                }
            }
        }
        // A FIFO entry pops when the LAST byte of the register that carries
        // `pop: true` has been clocked out. A driver that abandons the burst
        // after an earlier axis gets the same sample again next time, which is
        // what the silicon does with a read that never completed.
        if let Some(ptr) = self.pointer {
            if let Some((name, width)) = self
                .find_register(ptr)
                .filter(|r| r.fifo.as_ref().is_some_and(|f| f.pop))
                .map(|r| (r.name.clone(), r.width as usize))
            {
                if self.read_idx == width {
                    self.fifo_pop_after_read(&name);
                }
            }
        }
        byte
    }

    fn advance_time_us(&mut self, us: u64) {
        self.elapsed_us = self.elapsed_us.saturating_add(us);
        // A non-zero advance is the proof that this bus has an honest µs source
        // and that `data_ready` gating is meaningful here. Zero-length slices
        // (the central drive runs every slice) prove nothing either way.
        if us > 0 {
            self.time_source_seen = true;
        }
        self.advance_rule_time(us);
        // The part's own clock: every timer due at the new time fires, in
        // deadline order (see `TimerBank::due_by_timer`), before anything reads
        // a register. A device with no timers pays one `is_empty` check.
        //
        // ONE clock, two consumers: each firing runs its `on_fire` register
        // actions and THEN raises the Tier-2 `timer:<name>` event, so a rule
        // sees the registers that same firing already changed. The rule machine
        // holds no deadlines of its own and so cannot drift from this.
        if !self.timers.is_empty() {
            for (name, actions) in self.timers.due_by_timer(self.elapsed_us) {
                for action in &actions {
                    apply_timing_action(action, &mut self.reg_values);
                }
                // ⚠️ The FIFO fills BEFORE the `timer:` rules, so a rule
                // guarded on `fifo_len(samples)` — a watermark rule, the whole
                // reason a part has a FIFO — sees the sample this tick
                // produced rather than the previous one.
                self.fill_fifos_on_timer(&name);
                self.raise(Event::Timer { name }, 0);
            }
        }
        self.drain_timer_requests();
    }

    /// Tier 2: hand the bus whatever pin transitions the rules queued. Empty
    /// for every part that declares no `outputs:`.
    fn take_pin_drives(&mut self) -> Vec<(String, bool)> {
        match self.rules.as_mut() {
            Some(m) => m.take_pin_drives(),
            None => Vec::new(),
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

impl SimInput for GenericI2cDevice {
    fn input_channels(&self) -> &'static [InputChannel] {
        self.channels
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.slots.insert(key.to_string(), value);
        self.raise_and_settle(
            Event::Input {
                key: key.to_string(),
            },
            0,
        );
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id.clone());
        // Re-key the noise states so two identical devices on one bus diverge.
        for (key, n) in self.noise.iter_mut() {
            *n = ChannelNoise::new(0, &id, key, n.sigma(), n.bias(), n.tau_s());
        }
    }
}

/// Validate the static descriptor contract for the `i2c_device` primitive.
///
/// Kept separate from construction so a manifest can preflight every carried
/// pack without leaking runtime channel tables for leaves it does not attach.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    if descriptor.behavior.primitive != "i2c_device" {
        bail!(
            "declarative i2c kit requires behavior.primitive: i2c_device, got '{}'",
            descriptor.behavior.primitive
        );
    }
    let spec = descriptor
        .behavior
        .i2c
        .as_ref()
        .context("declarative i2c kit is missing behavior.i2c")?;
    validate_spec(spec)?;
    // A timer fires at registers BY NAME. A register-file device has no names,
    // so a timer there could never do anything; say so at load rather than
    // shipping a part whose clock is wired to nothing.
    let names: Vec<String> = spec.registers.iter().map(|r| r.name.clone()).collect();
    if !descriptor.behavior.timers.is_empty() && names.is_empty() {
        bail!(
            "behavior.timers needs a named register map: a timer fires at registers by name,              and this device declares commands or a register_file"
        );
    }
    validate_timers(
        &descriptor.behavior.timers,
        &names,
        &descriptor.behavior.rules,
    )?;
    // Derived channels are compiled here as well as at construction, so a
    // manifest preflight rejects a broken expression without building a device.
    let input_keys: Vec<String> = descriptor
        .metadata
        .as_ref()
        .map(|m| m.inputs.iter().map(|i| i.key.clone()).collect())
        .unwrap_or_default();
    let derived = compile_derived(&descriptor.behavior.derived, &input_keys)?;
    // A `source_from` table entry that names no channel would read 0.0 forever
    // on that mux setting — a converter silently reporting ground on one of its
    // inputs, which is exactly the kind of quiet wrong answer a twin exists to
    // refuse.
    for reg in spec.registers.iter().filter(|r| r.source_from.is_some()) {
        let sf = reg.source_from.as_ref().expect("filtered");
        for (value, key) in &sf.table {
            let known =
                input_keys.iter().any(|k| k == key) || derived.iter().any(|d| &d.name == key);
            if !known {
                bail!(
                    "register '{}' source_from maps field value {value} to '{key}', which is \
                     neither a declared input channel nor a derived channel",
                    reg.name
                );
            }
        }
    }
    // Tier 2: every expression must parse and every name a rule mentions must
    // be declared. Both are LOAD errors, so a typo in a generated part document
    // fails in manifest preflight rather than evaluating to a silent zero at
    // the first transaction (see `labwired_config::expr` on why evaluation is
    // deliberately total and validation deliberately is not).
    labwired_config::compile_rules(&descriptor.behavior.rules)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    super::declarative_gpio::validate_rule_names(descriptor)
}

/// A descriptor is exactly one shape (registers XOR commands XOR register_file),
/// and command devices with CRC framing must use even-width words (CRC is
/// computed per 16-bit word).
fn validate_spec(spec: &I2cSpec) -> Result<()> {
    // Pointer width: 1 (the universal 8-bit pointer) or 2 (EEPROM-style 16-bit
    // addressing). Anything else is a typo that would quietly eat data bytes as
    // address bytes.
    if !(1..=2).contains(&spec.pointer_width) {
        bail!(
            "behavior.i2c pointer_width {} unsupported (1 or 2)",
            spec.pointer_width
        );
    }
    // A register whose pointer the device can never produce answers nothing.
    if spec.pointer_width == 1 {
        for reg in &spec.registers {
            if reg.addr > 0xFF {
                bail!(
                    "register '{}' addr {:#06x} needs pointer_width: 2 — a one-byte pointer                      can never select it",
                    reg.name,
                    reg.addr
                );
            }
        }
    }
    if let Some(page) = spec.write_page {
        if page < 2 {
            bail!("behavior.i2c write_page {page} is not a page (2 or more bytes)");
        }
        if spec.register_file.is_none() {
            bail!(
                "behavior.i2c declares write_page but no register_file (page wrap is a                  memory-array behaviour)"
            );
        }
    }
    let has_regs = !spec.registers.is_empty();
    let has_cmds = !spec.commands.is_empty();
    let has_file = spec.register_file.is_some();
    match has_regs as u8 + has_cmds as u8 + has_file as u8 {
        0 => bail!("behavior.i2c declares none of registers / commands / register_file"),
        1 => {}
        _ => bail!(
            "behavior.i2c declares more than one of registers / commands / register_file \
             (a device is exactly one shape)"
        ),
    }
    // `observables` read the byte register file; `updates` drive a wide register.
    if !spec.observables.is_empty() && !has_file {
        bail!("behavior.i2c declares observables but no register_file (observables read it)");
    }
    if !spec.updates.is_empty() && !has_regs {
        bail!("behavior.i2c declares updates but no registers (updates drive a wide register)");
    }
    // A data_ready rule that names a register the device does not have would
    // silently never fire — the exact failure mode (a ready bit that never
    // appears) that hangs firmware inside a vendor poll loop. Reject it here.
    if !spec.data_ready.is_empty() && !has_regs {
        bail!(
            "behavior.i2c declares data_ready but no registers (data_ready gates a register bit)"
        );
    }
    for dr in &spec.data_ready {
        for (role, name) in [
            ("start_register", &dr.start_register),
            ("ready_register", &dr.ready_register),
        ] {
            if !spec.registers.iter().any(|r| &r.name == name) {
                bail!(
                    "data_ready '{}' {role} '{name}' is not a declared register",
                    dr.name
                );
            }
        }
        for name in &dr.clear_on_read {
            if !spec.registers.iter().any(|r| &r.name == name) {
                bail!(
                    "data_ready '{}' clear_on_read '{name}' is not a declared register",
                    dr.name
                );
            }
        }
        for name in &dr.clear_on_write {
            let Some(reg) = spec.registers.iter().find(|r| &r.name == name) else {
                bail!(
                    "data_ready '{}' clear_on_write '{name}' is not a declared register",
                    dr.name
                );
            };
            // A read-only register can never be written, so the clear could
            // never fire and firmware would hang on a flag that stays set —
            // the same silent-stall class `data_ready` was built to expose.
            if reg.access != I2cAccess::Rw {
                bail!(
                    "data_ready '{}' clear_on_write '{name}' is read-only — firmware could \
                     never clear the flag",
                    dr.name
                );
            }
        }
        if dr.start_mask == 0 || dr.ready_mask == 0 {
            bail!(
                "data_ready '{}' has an empty start_mask or ready_mask (it could never fire)",
                dr.name
            );
        }
        // The start bits must survive a firmware write, or the conversion could
        // never be started; the ready bits must NOT, or firmware could forge
        // readiness. Both are `write_mask` questions on the named registers.
        let start_reg = spec
            .registers
            .iter()
            .find(|r| r.name == dr.start_register)
            .expect("checked above");
        if start_reg.access != labwired_config::RegisterAccess::Rw {
            bail!(
                "data_ready '{}' start_register '{}' is read-only — firmware could never \
                 start a conversion",
                dr.name,
                dr.start_register
            );
        }
        if let Some(mask) = start_reg.write_mask {
            if dr.start_mask & !mask != 0 {
                bail!(
                    "data_ready '{}' start_mask {:#x} includes bits '{}' write_mask {:#x} \
                     protects — firmware could never start a conversion",
                    dr.name,
                    dr.start_mask,
                    dr.start_register,
                    mask
                );
            }
        }
        let ready_reg = spec
            .registers
            .iter()
            .find(|r| r.name == dr.ready_register)
            .expect("checked above");
        let writable = match (ready_reg.access, ready_reg.write_mask) {
            (labwired_config::RegisterAccess::R, _) => 0,
            (labwired_config::RegisterAccess::Rw, Some(mask)) => mask,
            (labwired_config::RegisterAccess::Rw, None) => u32::MAX,
        };
        if dr.ready_mask & writable != 0 {
            bail!(
                "data_ready '{}' ready_mask {:#x} overlaps bits firmware may write in '{}' — \
                 the status bit must be model-owned (narrow its write_mask)",
                dr.name,
                dr.ready_mask,
                dr.ready_register
            );
        }
    }
    // A bank-carrying register on a device with no bank select could never
    // decode; a bank select with no banked register is dead configuration.
    // Either way the descriptor means something it does not do, so reject it.
    if spec.page_register.is_none() && spec.registers.iter().any(|r| r.page.is_some()) {
        bail!("a register declares a `page` but the device declares no `page_register`");
    }
    if spec.page_register.is_some() && !spec.registers.iter().any(|r| r.page.is_some()) {
        bail!("`page_register` is declared but no register names a `page`");
    }
    // `source_from` is a mux over another register's bit-field: the register has
    // to exist, the mask has to select something, and a table that maps nothing
    // is a mux that never switches.
    for reg in spec.registers.iter().filter(|r| r.source_from.is_some()) {
        let sf = reg.source_from.as_ref().expect("filtered");
        if !spec.registers.iter().any(|r| r.name == sf.register) {
            bail!(
                "register '{}' source_from register '{}' is not a declared register",
                reg.name,
                sf.register
            );
        }
        if sf.mask == 0 {
            bail!(
                "register '{}' source_from mask is 0 — the mux could never switch",
                reg.name
            );
        }
        if sf.table.is_empty() {
            bail!(
                "register '{}' source_from declares an empty table — it selects nothing",
                reg.name
            );
        }
    }
    // A field's `scale_from` reads the same register file a register's does.
    for reg in &spec.registers {
        for f in &reg.fields {
            for sf in &f.scale_from {
                if !spec.registers.iter().any(|r| r.name == sf.register) {
                    bail!(
                        "register '{}' field '{}' scale_from register '{}' is not a declared \
                         register",
                        reg.name,
                        f.source,
                        sf.register
                    );
                }
            }
        }
    }
    // Hybrid auto-increment. Every failure here is silent otherwise: a jump on a
    // device whose pointer never walks does nothing, two jumps from one address
    // make the walk depend on declaration order, and a jump to itself parks the
    // pointer forever on one byte.
    if !spec.auto_increment_map.is_empty() {
        if !spec.auto_increment {
            bail!(
                "behavior.i2c declares auto_increment_map but auto_increment is false — the \
                 pointer never walks, so the jump could never be taken"
            );
        }
        for (i, m) in spec.auto_increment_map.iter().enumerate() {
            if m.from == m.to {
                bail!(
                    "auto_increment_map entry {i} jumps {:#06x} to itself — the pointer would \
                     never leave that byte",
                    m.from
                );
            }
            if spec.auto_increment_map[..i]
                .iter()
                .any(|e| e.from == m.from)
            {
                bail!(
                    "auto_increment_map declares {:#06x} twice — which jump wins would depend \
                     on declaration order",
                    m.from
                );
            }
        }
    }
    for pc in spec.registers.iter().filter_map(|r| r.popcount.as_ref()) {
        for name in &pc.registers {
            if !spec.registers.iter().any(|r| &r.name == name) {
                bail!("popcount source '{name}' is not a declared register");
            }
        }
    }
    // `zero_when` power-gates. A mistake here is the silent-stall class the
    // `data_ready` validation exists for: the gate would never fire (or could
    // never be lifted), and the model would look faithful while behaving like
    // the un-gated part it replaced.
    for reg in spec.registers.iter() {
        if reg.zero_when.is_some() && reg.zero_unless.is_some() {
            bail!(
                "register '{}' declares both zero_when and zero_unless — a gate has one \
                 polarity, and the two together say the part is both on and off",
                reg.name
            );
        }
    }
    for reg in spec
        .registers
        .iter()
        .filter(|r| r.zero_when.is_some() || r.zero_unless.is_some())
    {
        // ONE branch for both polarities: `zero_unless` needs firmware to be
        // able to SET the bit and `zero_when` to be able to CLEAR it, and both
        // are the same question — can firmware write it at all.
        let (key, z) = match (&reg.zero_when, &reg.zero_unless) {
            (Some(z), _) => ("zero_when", z),
            (_, Some(z)) => ("zero_unless", z),
            _ => unreachable!("filtered"),
        };
        let gate = match spec.registers.iter().find(|r| r.name == z.register) {
            Some(g) => g,
            None => bail!(
                "register '{}' {key} register '{}' is not a declared register",
                reg.name,
                z.register
            ),
        };
        if z.mask == 0 {
            bail!(
                "register '{}' {key} mask is 0 — the gate could never fire",
                reg.name
            );
        }
        if z.register == reg.name {
            bail!(
                "register '{}' {key} gates itself — the gate would erase the very bits \
                 that control it",
                reg.name
            );
        }
        // Firmware must be able to LIFT the gate, or the register is dead: a
        // read-only (or write-masked) gate bit no driver can clear is a part
        // that can never report a measurement.
        let writable = match (gate.access, gate.write_mask) {
            (labwired_config::RegisterAccess::R, _) => 0,
            (labwired_config::RegisterAccess::Rw, Some(mask)) => mask,
            (labwired_config::RegisterAccess::Rw, None) => u32::MAX,
        };
        if z.mask & !writable != 0 {
            bail!(
                "register '{}' {key} mask {:#x} includes bits firmware cannot write in '{}' \
                 — the part could never be powered on",
                reg.name,
                z.mask,
                z.register
            );
        }
    }
    for t in &spec.indexed_tables {
        for (role, name) in [
            ("index_register", &t.index_register),
            ("strobe_register", &t.strobe_register),
            ("data_register", &t.data_register),
        ] {
            if !spec.registers.iter().any(|r| &r.name == name) {
                bail!(
                    "indexed_table '{}' {role} '{name}' is not a declared register",
                    t.name
                );
            }
        }
        if t.strobe_mask == 0 {
            bail!(
                "indexed_table '{}' has an empty strobe_mask (the fetch could never be observed)",
                t.name
            );
        }
        // The strobe is the device's answer, not firmware's: if firmware could
        // write those bits it could forge a completed fetch, and a driver that
        // never really waited would appear to work.
        let strobe = spec
            .registers
            .iter()
            .find(|r| r.name == t.strobe_register)
            .expect("checked above");
        let writable = match (strobe.access, strobe.write_mask) {
            (labwired_config::RegisterAccess::R, _) => 0,
            (labwired_config::RegisterAccess::Rw, Some(mask)) => mask,
            (labwired_config::RegisterAccess::Rw, None) => u32::MAX,
        };
        if t.strobe_mask & writable != 0 {
            bail!(
                "indexed_table '{}' strobe_mask {:#x} overlaps bits firmware may write in '{}' — \
                 the strobe must be model-owned (narrow its write_mask)",
                t.name,
                t.strobe_mask,
                t.strobe_register
            );
        }
        // The master arms the fetch by writing the strobe, so it must be
        // writable at all.
        if strobe.access != labwired_config::RegisterAccess::Rw {
            bail!(
                "indexed_table '{}' strobe_register '{}' is read-only — firmware could never \
                 arm a fetch",
                t.name,
                t.strobe_register
            );
        }
    }
    if let Some(rf) = &spec.register_file {
        if rf.size == 0 || rf.size > 65536 {
            bail!("register_file.size {} outside 1..=65536", rf.size);
        }
        for &off in rf.reset.keys() {
            if off as usize >= rf.size {
                bail!(
                    "register_file reset offset {off:#x} outside the file (size {:#x})",
                    rf.size
                );
            }
        }
        if let AutoIncrement::WhenFieldSet { addr, .. } = &rf.auto_increment {
            if *addr as usize >= rf.size {
                bail!("auto_increment enable register {addr:#x} outside the register file");
            }
        }
        for o in &spec.observables {
            let span = o.value.u12_compose.lo_rel.max(o.value.u12_compose.hi_rel) as usize;
            let last =
                o.base as usize + o.stride as usize * (o.channels.max(1) as usize - 1) + span;
            if last >= rf.size {
                bail!(
                    "observable '{}' channel block ends at {last:#x}, outside register_file (size {:#x})",
                    o.name,
                    rf.size
                );
            }
        }
    }
    // Every `read_complete` update must name a real register (by pointer or name).
    for u in &spec.updates {
        let rc = &u.trigger.read_complete;
        let ok = match (&rc.pointer, &rc.register) {
            (Some(p), _) => spec.registers.iter().any(|r| r.addr == *p),
            (None, Some(name)) => spec.registers.iter().any(|r| &r.name == name),
            (None, None) => false,
        };
        if !ok {
            bail!("update read_complete trigger does not name a declared register");
        }
    }
    if !spec.commands.is_empty() && !matches!(spec.code_width, 1 | 2) {
        bail!(
            "command device has code_width {} — only 1 (single-byte opcode) or 2 \
             (16-bit opcode) are supported",
            spec.code_width
        );
    }
    if let Some(c) = spec.crc8 {
        if c.covers == Crc8Covers::Response {
            for cmd in &spec.commands {
                for word in &cmd.response {
                    if word.width % 2 != 0 {
                        bail!(
                            "command '{}' has an odd-width response word ({}); CRC-8 framing is \
                             computed per 16-bit word",
                            cmd.name,
                            word.width
                        );
                    }
                }
            }
        }
        // An SMBus PEC covers `[addr·W, command, addr·R, data…]`, so it is a
        // property of a COMMAND transaction. On a register device there is no
        // command byte to checksum and the key would parse and do nothing.
        if c.covers == Crc8Covers::Transaction && spec.commands.is_empty() {
            bail!(
                "behavior.i2c declares crc8.covers: transaction but no commands — an SMBus PEC \
                 covers the command byte the master wrote, so it needs a command device"
            );
        }
    }
    Ok(())
}

// ─── Discovery-channel leaking ─────────────────────────────────────────────

/// Leak the descriptor's `metadata.inputs` into a `'static` channel table
/// (`InputChannel` requires `'static` strings). One table per call — the kit
/// leaks once and shares it; tests leak per device.
pub(crate) fn leak_channels(descriptor: &DeviceDescriptor) -> &'static [InputChannel] {
    let inputs = descriptor
        .metadata
        .as_ref()
        .map(|m| m.inputs.as_slice())
        .unwrap_or(&[]);
    let channels: Vec<InputChannel> = inputs
        .iter()
        .map(|i| InputChannel {
            key: Box::leak(i.key.clone().into_boxed_str()),
            label: Box::leak(i.label.clone().into_boxed_str()),
            unit: Box::leak(i.unit.clone().into_boxed_str()),
            min: i.min,
            max: i.max,
        })
        .collect();
    Box::leak(channels.into_boxed_slice())
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

/// A [`PeripheralKit`] backed by a declarative `i2c_device` descriptor — one
/// instance per YAML device. `metadata()` must hand back a `&'static
/// KitMetadata`, so `from_yaml` builds it once and leaks it (the kit is itself
/// a long-lived registry entry, so the leak is bounded by the device count).
///
/// Phase 1 ships the machinery but registers no real parts: no instance is
/// added to [`crate::peripherals::kit::registry::KITS`], so the offline
/// peripherals manifest is unchanged.
pub struct DeclarativeI2cKit {
    descriptor: DeviceDescriptor,
    channels: &'static [InputChannel],
    metadata: &'static KitMetadata,
}

impl DeclarativeI2cKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;
        let spec = descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c kit is missing behavior.i2c")?;
        let default_address = spec.default_address;

        let channels = leak_channels(&descriptor);
        let metadata = leak_metadata(&descriptor, channels, default_address);
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }
}

/// Map a descriptor's `config_keys[].ty` string onto a [`ConfigType`].
/// Unknown spellings fall back to `Str` (the most permissive display type).
pub(super) fn config_type_from_str(ty: &str) -> ConfigType {
    match ty {
        "int" => ConfigType::Int,
        "float" => ConfigType::Float,
        "bool" => ConfigType::Bool,
        _ => ConfigType::Str,
    }
}

/// Derive a `&'static KitMetadata` from the descriptor's display metadata.
fn leak_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
    default_address: u8,
) -> &'static KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative I²C device.".to_string());
    // Long-form detail: explicit `metadata.detail` if given, else the summary
    // (the pre-existing declarative-kit fallback).
    let detail = meta
        .and_then(|m| m.detail.clone())
        .unwrap_or_else(|| summary.clone());

    // Config keys: an explicit `metadata.config_keys` is taken as the COMPLETE
    // set (it may list `i2c_address` itself); otherwise synthesise the lone
    // `i2c_address` key from the default address.
    let declared_keys = meta.map(|m| m.config_keys.as_slice()).unwrap_or(&[]);
    let config_keys: &'static [ConfigKey] = if declared_keys.is_empty() {
        Box::leak(
            vec![ConfigKey {
                name: "i2c_address",
                ty: ConfigType::Int,
                doc: leak(format!(
                    "7-bit slave address. Defaults to 0x{default_address:02x}."
                )),
            }]
            .into_boxed_slice(),
        )
    } else {
        Box::leak(
            declared_keys
                .iter()
                .map(|k| ConfigKey {
                    name: leak(k.name.clone()),
                    ty: config_type_from_str(&k.ty),
                    doc: leak(k.doc.clone()),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    };

    // Labs: mirror any declared starter labs verbatim.
    let declared_labs = meta.map(|m| m.labs.as_slice()).unwrap_or(&[]);
    let labs: &'static [LabRef] = Box::leak(
        declared_labs
            .iter()
            .map(|l| LabRef {
                board_id: leak(l.board_id.clone()),
                chip: leak(l.chip.clone()),
                example_dir: leak(l.example_dir.clone()),
                demo_elf: leak(l.demo_elf.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary),
        detail: leak(detail),
        transport: Transport::I2c,
        category: Category::I2c,
        config_keys,
        labs,
        inputs: channels,
    }))
}

/// [`leak_metadata`]'s GPIO twin: a pins-only descriptor has no I²C address, so
/// there is no synthesised `i2c_address` key and the transport is the GPIO
/// group. Everything else — label, summary, detail, `config_keys`, labs and
/// stimulus channels — is mirrored from the descriptor exactly the same way, so
/// a part reads identically in the manifest whichever primitive it uses.
pub(crate) fn leak_gpio_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
) -> &'static KitMetadata {
    leak_pinlike_metadata(
        descriptor,
        channels,
        Transport::GpioGroup,
        Category::Gpio,
        "Declarative GPIO device.",
    )
}

/// Same, for a declarative `uart_device`: a part with no register map and no
/// pads, whose whole interface is the byte stream. It takes the SAME path as
/// the GPIO one because the only thing that differs is the transport label the
/// manifest shows — writing it twice is how the two would come to disagree
/// about which `config_keys` a descriptor may declare.
pub(crate) fn leak_uart_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
) -> &'static KitMetadata {
    leak_pinlike_metadata(
        descriptor,
        channels,
        Transport::Uart,
        Category::Uart,
        "Declarative UART device.",
    )
}

/// The shared body: a descriptor with no `i2c:`/`spi:` block, so there is no
/// address to synthesise a `config_keys` entry from and the declared list is
/// taken as the complete set.
fn leak_pinlike_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
    transport: Transport,
    category: Category,
    default_summary: &str,
) -> &'static KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| default_summary.to_string());
    let detail = meta
        .and_then(|m| m.detail.clone())
        .unwrap_or_else(|| summary.clone());
    let config_keys: &'static [ConfigKey] = Box::leak(
        meta.map(|m| m.config_keys.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|k| ConfigKey {
                name: leak(k.name.clone()),
                ty: config_type_from_str(&k.ty),
                doc: leak(k.doc.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let labs: &'static [LabRef] = Box::leak(
        meta.map(|m| m.labs.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|l| LabRef {
                board_id: leak(l.board_id.clone()),
                chip: leak(l.chip.clone()),
                example_dir: leak(l.example_dir.clone()),
                demo_elf: leak(l.demo_elf.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary),
        detail: leak(detail),
        transport,
        category,
        config_keys,
        labs,
        inputs: channels,
    }))
}

impl PeripheralKit for DeclarativeI2cKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let spec = self
            .descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c kit is missing behavior.i2c")?;
        let address = ctx.i2c_address_or(spec.default_address)?;
        let mut device =
            GenericI2cDevice::from_descriptor(&self.descriptor, address, self.channels)?;
        // Honour `config:` overrides that seed an input channel (e.g. a `lux`
        // seed), matching how a hand-written kit seeded its initial reading.
        device.seed_from_config(|key| ctx.config_f64(key));
        // `noise_sigma_key`: a `config:` knob that sets a channel's noise sigma.
        // Named per channel, so one key can reach a whole channel SET — an
        // IMU's six axes quote one datasheet noise figure, and the placement
        // says `noise_sigma: 0.02` once.
        for input in self
            .descriptor
            .metadata
            .iter()
            .flat_map(|m| m.inputs.iter())
            .filter(|i| i.noise_sigma_key.is_some())
        {
            let key = input.noise_sigma_key.as_deref().expect("filtered above");
            if let Some(sigma) = ctx.config_f64(key) {
                device.set_channel_noise_sigma(&input.key, sigma);
            }
        }
        // Tier 2: bind `outputs:` roles to pads BEFORE the device goes in, so a
        // wiring error is reported against the placement rather than leaving a
        // device attached with an interrupt line that goes nowhere.
        ctx.bind_output_pins(&self.descriptor)?;
        ctx.attach_i2c_device(Box::new(device))
    }
}

// ─── Registry statics ──────────────────────────────────────────────────────
//
// A `DeclarativeI2cKit` is parsed from YAML at runtime, but the registry
// (`registry::KITS`) is a const slice of `&'static dyn PeripheralKit`. A
// `static LazyLock<DeclarativeI2cKit>` is the const-initialisable cell that
// bridges the two: the descriptor is parsed once on first access, and the
// `PeripheralKit` impl below forwards through it. Real parts get one static
// each here and one line in `registry::KITS`; the descriptor lives entirely in
// `configs/devices/*.yaml`.

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeI2cKit> {
    fn metadata(&self) -> &'static KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Sensirion SHT31 temperature + humidity sensor (declarative `sht31.yaml`).
pub static SHT31_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("sht31").expect("sht31 descriptor is embedded"),
    )
    .expect("sht31.yaml is a valid declarative i2c descriptor")
});

/// Microchip MCP9808 temperature sensor (declarative `mcp9808.yaml`).
pub static MCP9808_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("mcp9808").expect("mcp9808 descriptor is embedded"),
    )
    .expect("mcp9808.yaml is a valid declarative i2c descriptor")
});

/// ROHM BH1750 ambient-light sensor (declarative `bh1750.yaml`).
pub static BH1750_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("bh1750").expect("bh1750 descriptor is embedded"),
    )
    .expect("bh1750.yaml is a valid declarative i2c descriptor")
});

/// Vishay VEML7700 ambient-light sensor (declarative `veml7700.yaml`). Migrated
/// from the hand-written [`super::veml7700::Veml7700`] model, which now survives
/// only as the byte-parity oracle (see `veml7700_parity.rs`). The register-pointer
/// wire protocol, the gain × integration-time resolution table, and the manifest
/// metadata all live in the descriptor.
pub static VEML7700_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("veml7700").expect("veml7700 descriptor is embedded"),
    )
    .expect("veml7700.yaml is a valid declarative i2c descriptor")
});

/// TI TMP102 temperature sensor (declarative `tmp102.yaml`, register-pointer +
/// self-driving drift). Migrated from the hand-written
/// [`super::super::esp32s3::tmp102::Tmp102`] model, which now survives only as
/// the byte-parity oracle (see `pca9685_tmp102_parity.rs`).
pub static TMP102_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("tmp102").expect("tmp102 descriptor is embedded"),
    )
    .expect("tmp102.yaml is a valid declarative i2c descriptor")
});

/// NXP PCA9685 16-channel PWM controller (declarative `pca9685.yaml`,
/// byte-addressable register file + `servo_angle` observable). Migrated from the
/// hand-written [`super::pca9685::Pca9685`] model, which now survives only as the
/// byte-parity oracle (see `pca9685_tmp102_parity.rs`).
pub static PCA9685_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").expect("pca9685 descriptor is embedded"),
    )
    .expect("pca9685.yaml is a valid declarative i2c descriptor")
});

/// Vishay VCNL4010 proximity + ambient sensor (declarative `vcnl4010.yaml`).
/// Written declaratively from the start — there is no hand-written model to
/// migrate from and none is needed: the part is a register map plus two input
/// channels. Its address 0x13 is fixed in silicon, so more than one on a bus
/// requires a [`super::tca9548a::Tca9548a`] switch; see
/// `tests/vcnl4010_bay_occupancy.rs` for that topology driven end to end.
pub static VCNL4010_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("vcnl4010").expect("vcnl4010 descriptor is embedded"),
    )
    .expect("vcnl4010.yaml is a valid declarative i2c descriptor")
});

/// NXP PCF8574 8-bit I/O expander (declarative `pcf8574.yaml`) — the first
/// TIER-2 port: an I²C write moves eight PADS, through `outputs:` and rules.
///
/// Migrated from the hand-written [`super::pcf8574::Pcf8574`], which is DELETED
/// rather than kept as an oracle; `tests/pcf8574_migration_parity.rs` holds the
/// transcript it produced, which this descriptor reproduces byte for byte, and
/// names the one thing that is new (the pads move at all).
pub static PCF8574_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("pcf8574").expect("pcf8574 descriptor is embedded"),
    )
    .expect("pcf8574.yaml is a valid declarative i2c descriptor")
});

/// ST VL53L0X laser time-of-flight sensor (declarative `vl53l0x.yaml`).
///
/// Migrated from a hand-written model that is DELETED rather than kept as a
/// parity oracle, because one behaviour deliberately changed: that model's
/// ready flag latched on the first start with no conversion time, where this
/// one follows ST's 33 ms timing budget and clears on acknowledge. An oracle
/// asserting the old behaviour would be asserting the bug.
/// `tests/vl53l0x_migration_parity.rs` holds the transcripts that must stay
/// identical and states the one that must not.
pub static VL53L0X_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("vl53l0x").expect("vl53l0x descriptor is embedded"),
    )
    .expect("vl53l0x.yaml is a valid declarative i2c descriptor")
});

/// ams AS5600 magnetic rotary encoder (declarative `as5600.yaml`).
///
/// Migrated from a hand-written model that is DELETED rather than kept as a
/// parity oracle: it discarded every configuration write, so an oracle
/// asserting the old behaviour would be asserting the bug.
/// `tests/as5600_migration_parity.rs` holds the transcripts that must stay
/// identical and states the one that must not.
pub static AS5600_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("as5600").expect("as5600 descriptor is embedded"),
    )
    .expect("as5600.yaml is a valid declarative i2c descriptor")
});

/// Sensirion SHT30 temperature + humidity sensor (declarative `sht30.yaml`,
/// command device with Sensirion CRC-8 framing). Migrated from a hand-written
/// model that answered EVERY opcode with a measurement frame; see
/// `tests/sht30_migration_parity.rs`.
/// Microchip CAP1188 capacitive touch controller (declarative `cap1188.yaml`).
pub static CAP1188_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("cap1188").expect("cap1188 descriptor is embedded"),
    )
    .expect("cap1188.yaml is a valid declarative i2c descriptor")
});

/// Bosch BMI270 6-axis IMU (declarative `bmi270.yaml`).
pub static BMI270_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("bmi270").expect("bmi270 descriptor is embedded"),
    )
    .expect("bmi270.yaml is a valid declarative i2c descriptor")
});

/// Sensirion SCD41 CO₂ sensor (declarative `scd41.yaml`).
pub static SCD41_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("scd41").expect("scd41 descriptor is embedded"),
    )
    .expect("scd41.yaml is a valid declarative i2c descriptor")
});

/// Sensirion SGP41 VOC/NOx sensor (declarative `sgp41.yaml`).
pub static SGP41_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("sgp41").expect("sgp41 descriptor is embedded"),
    )
    .expect("sgp41.yaml is a valid declarative i2c descriptor")
});

pub static SHT30_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("sht30").expect("sht30 descriptor is embedded"),
    )
    .expect("sht30.yaml is a valid declarative i2c descriptor")
});

/// Atmel AT24C256 serial EEPROM (declarative `at24c256.yaml`) — the
/// byte-addressable register file with a 16-bit pointer (`pointer_width: 2`)
/// and page-wrapped writes (`write_page`). Migrated from a hand-written model
/// whose sequential write ran straight through the array; see
/// `tests/at24c256_migration_parity.rs`.
pub static AT24C256_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("at24c256").expect("at24c256 descriptor is embedded"),
    )
    .expect("at24c256.yaml is a valid declarative i2c descriptor")
});

/// TI TMP117 high-accuracy temperature sensor (declarative `tmp117.yaml`,
/// register-pointer device with 16-bit big-endian words). Migrated from a
/// hand-written model whose DATA_READY bit tracked host stimulus rather than
/// conversion, so firmware that polled it before driving one spun forever; see
/// `tests/tmp117_migration_parity.rs`.
pub static TMP117_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("tmp117").expect("tmp117 descriptor is embedded"),
    )
    .expect("tmp117.yaml is a valid declarative i2c descriptor")
});

/// Maxim DS3231 real-time clock (declarative `ds3231.yaml`).
///
/// Migrated from the hand-written [`super::ds3231::Ds3231`], which is DELETED
/// rather than kept as an oracle; `tests/ds3231_migration_parity.rs` holds the
/// transcript it produced and names each deliberate difference — the seven time
/// registers are now ONE settable instant (`calendar:`), the temperature word
/// carries its real quarter-degrees, and the alarm registers accept a write.
pub static DS3231_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("ds3231").expect("ds3231 descriptor is embedded"),
    )
    .expect("ds3231.yaml is a valid declarative i2c descriptor")
});

/// Analog Devices ADXL345 accelerometer on I²C (declarative `adxl345.yaml`).
///
/// Migrated from the hand-written [`super::adxl345::Adxl345`], which is DELETED
/// rather than kept as an oracle; `tests/adxl345_migration_parity.rs` holds the
/// transcript it produced. The SPI variant of the same silicon is
/// [`super::declarative_spi::ADXL345_KIT`] (`adxl345_spi.yaml`).
pub static ADXL345_I2C_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("adxl345").expect("adxl345 descriptor is embedded"),
    )
    .expect("adxl345.yaml is a valid declarative i2c descriptor")
});

/// InvenSense MPU-6050 6-axis IMU (declarative `mpu6050.yaml`).
///
/// Migrated from the hand-written [`super::mpu6050::Mpu6050`], which is DELETED
/// rather than kept as an oracle: keeping it would mean keeping an oracle that
/// asserts its own bug. That model addressed the measurement block as
/// `(reg - 0x3B) / 2`, so it had no TEMP_OUT and every gyro axis sat one
/// register pair below its datasheet address — six of the fourteen bytes every
/// driver burst-reads. `tests/mpu6050_migration_parity.rs` holds the transcript
/// over the registers it got right and pins the fix for the rest.
pub static MPU6050_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("mpu6050").expect("mpu6050 descriptor is embedded"),
    )
    .expect("mpu6050.yaml is a valid declarative i2c descriptor")
});

/// TI INA219 current / bus-voltage monitor (declarative `ina219.yaml`). The
/// first descriptor to use `behavior.derived`: its POWER register is the
/// product of two stimulus channels. Migrated from a hand-written model whose
/// bus voltage TRUNCATED to the 4 mV LSB and whose POWER truncated an
/// intermediate to whole milliwatts; see `tests/ina219_migration_parity.rs`.
pub static INA219_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("ina219").expect("ina219 descriptor is embedded"),
    )
    .expect("ina219.yaml is a valid declarative i2c descriptor")
});

/// TI ADS1115 16-bit ADC (declarative `ads1115.yaml`) — the `source_from`
/// multiplexer: CONVERSION reports whichever input CONFIG's MUX bits select, at
/// the full scale its PGA bits select. Migrated from a hand-written model that
/// answered all four differential pairs with AIN0; see
/// `tests/ads1115_migration_parity.rs`.
pub static ADS1115_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("ads1115").expect("ads1115 descriptor is embedded"),
    )
    .expect("ads1115.yaml is a valid declarative i2c descriptor")
});

/// NXP MMA8451Q accelerometer (declarative `mma8451q.yaml`) — the
/// left-justified 14-bit field with a per-field `scale_from`, and the inverted
/// `zero_unless` standby gate. Migrated from a hand-written model that wrapped
/// full positive scale to full NEGATIVE scale; see
/// `tests/mma8451q_migration_parity.rs`.
pub static MMA8451Q_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("mma8451q").expect("mma8451q descriptor is embedded"),
    )
    .expect("mma8451q.yaml is a valid declarative i2c descriptor")
});

/// NXP FXOS8700CQ 6-axis sensor (declarative `fxos8700.yaml`) — the
/// `auto_increment_map` hybrid jump (0x06 → 0x33). Migrated from a hand-written
/// model that invented a "grazing cow" pose no stimulus had driven; see
/// `tests/fxos8700_migration_parity.rs`.
pub static FXOS8700_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("fxos8700").expect("fxos8700 descriptor is embedded"),
    )
    .expect("fxos8700.yaml is a valid declarative i2c descriptor")
});

/// Melexis MLX90614 IR thermometer (declarative `mlx90614.yaml`) — the SMBus
/// command device: a little-endian response word and a PEC over the whole
/// addressed transaction. Migrated from a hand-written model that answered
/// every command byte with a temperature; see
/// `tests/mlx90614_migration_parity.rs`.
pub static MLX90614_KIT: LazyLock<DeclarativeI2cKit> = LazyLock::new(|| {
    DeclarativeI2cKit::from_yaml(
        labwired_config::embedded_device_yaml("mlx90614").expect("mlx90614 descriptor is embedded"),
    )
    .expect("mlx90614.yaml is a valid declarative i2c descriptor")
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::components::sensirion::{crc8 as sensirion_crc8, encode_words};

    /// Register-mode fixture: a fictional light + temperature sensor exercising
    /// LE + BE words, an rw config register, source+encode, and scale_from.
    const REGISTER_FIXTURE: &str = include_str!("declarative_i2c_fixture.yaml");

    /// Command-mode fixture (inline): a Sensirion-shaped device. Kept inline
    /// because a descriptor YAML is exactly one device (register XOR command),
    /// and the on-disk fixture demonstrates the register schema.
    const COMMAND_FIXTURE: &str = r#"
type: test_i2c_command_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x62
    crc8: { poly: 0x31, init: 0xFF }
    commands:
      - name: start_periodic
        code: 0x21B1
      - name: get_data_ready
        code: 0xE4B8
        response:
          - { const: 0x8006 }
      - name: read_measurement
        code: 0xEC05
        response:
          - { source: co2, width: 2 }
          - { source: temperature, width: 2, encode: { scale: 372.771428, offset: 16776.75 } }
      - name: set_offset
        code: 0x241D
        params_words: 1
      - name: measure_single_shot
        code: 0x219D
        delay_us: 5000
        response:
          - { source: co2, width: 2 }
metadata:
  inputs:
    - { key: co2, label: "CO2", unit: ppm, min: 0, max: 40000, default: 450 }
    - { key: temperature, label: "Temperature", unit: "°C", min: -45, max: 130, default: 22 }
"#;

    /// Single-byte-opcode command fixture (inline): a BH1750-shaped device.
    /// `code_width: 1`, no CRC, one 16-bit BE response word, plus write-only
    /// power/reset opcodes that queue no response.
    const CODE_WIDTH_1_FIXTURE: &str = r#"
type: test_i2c_code_width_1_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x23
    code_width: 1
    commands:
      - name: power_on
        code: 0x01
      - name: reset
        code: 0x07
      - name: cont_hres
        code: 0x10
        response:
          - { source: lux, width: 2, encode: { scale: 1.2 } }
metadata:
  inputs:
    - { key: lux, label: "Illuminance", unit: lx, min: 0, max: 100000, default: 600 }
"#;

    fn reg_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(REGISTER_FIXTURE, 0).unwrap()
    }
    fn cmd_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(COMMAND_FIXTURE, 0).unwrap()
    }
    fn cw1_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(CODE_WIDTH_1_FIXTURE, 0).unwrap()
    }

    /// Send a single-byte opcode.
    fn send_byte_cmd(d: &mut GenericI2cDevice, code: u8) {
        d.start();
        d.write(code);
    }

    /// Point at `reg` and read `width` bytes.
    fn read_reg(d: &mut GenericI2cDevice, reg: u8, width: usize) -> Vec<u8> {
        d.start();
        d.write(reg);
        d.start(); // repeated START into the read phase
        (0..width).map(|_| d.read()).collect()
    }

    fn send_cmd(d: &mut GenericI2cDevice, code: u16) {
        d.start();
        d.write((code >> 8) as u8);
        d.write((code & 0xFF) as u8);
    }

    fn read_bytes(d: &mut GenericI2cDevice, n: usize) -> Vec<u8> {
        d.start();
        (0..n).map(|_| d.read()).collect()
    }

    // ── addresses / mode ───────────────────────────────────────────────────

    #[test]
    fn register_fixture_defaults_to_declared_address() {
        assert_eq!(reg_dev().address(), 0x40);
    }

    #[test]
    fn command_fixture_defaults_to_declared_address() {
        assert_eq!(cmd_dev().address(), 0x62);
    }

    #[test]
    fn explicit_address_overrides_default() {
        let d = GenericI2cDevice::from_yaml(REGISTER_FIXTURE, 0x55).unwrap();
        assert_eq!(d.address(), 0x55);
    }

    // ── register mode: streaming reads LE and BE ───────────────────────────

    #[test]
    fn light_register_reads_little_endian() {
        // LIGHT (0x01) sources `lux` (default 450), gain 1× ⇒ 450 counts, LE.
        let mut d = reg_dev();
        let b = read_reg(&mut d, 0x01, 2);
        let word = (b[0] as u16) | ((b[1] as u16) << 8); // LE decode
        assert_eq!(word, 450, "LE low byte first: {b:02x?}");
        assert_eq!(b, vec![0xC2, 0x01]);
    }

    #[test]
    fn temp_register_reads_big_endian() {
        // TEMP (0x02) sources `temperature` (default 22) with scale 100 ⇒ 2200
        // centi-°C, big-endian.
        let mut d = reg_dev();
        let b = read_reg(&mut d, 0x02, 2);
        let word = ((b[0] as u16) << 8) | b[1] as u16; // BE decode
        assert_eq!(word, 2200, "BE high byte first: {b:02x?}");
        assert_eq!(b, vec![0x08, 0x98]);
    }

    // ── register mode: rw write accumulation + read-back ────────────────────

    #[test]
    fn rw_config_register_accumulates_and_reads_back() {
        let mut d = reg_dev();
        // Write CONFIG (0x00) = 0x0002 little-endian (low, high).
        d.start();
        d.write(0x00);
        d.write(0x02);
        d.write(0x00);
        d.stop();
        let b = read_reg(&mut d, 0x00, 2);
        let word = (b[0] as u16) | ((b[1] as u16) << 8);
        assert_eq!(word, 0x0002, "rw register round-trips its written value");
    }

    // ── register mode: scale_from bit-field scaling ─────────────────────────

    #[test]
    fn scale_from_field_selects_light_gain() {
        let mut d = reg_dev();
        // Default gain field 0 ⇒ ×1 ⇒ 450 counts.
        let base = {
            let b = read_reg(&mut d, 0x01, 2);
            (b[0] as u16) | ((b[1] as u16) << 8)
        };
        assert_eq!(base, 450);
        // Program CONFIG gain field = 2 (bits [1:0]) ⇒ ×4 ⇒ 1800 counts.
        d.start();
        d.write(0x00);
        d.write(0x02);
        d.write(0x00);
        d.stop();
        let scaled = {
            let b = read_reg(&mut d, 0x01, 2);
            (b[0] as u16) | ((b[1] as u16) << 8)
        };
        assert_eq!(scaled, 1800, "gain field 2 ⇒ ×4 scale");
    }

    // ── register mode: set_input round-trip ────────────────────────────────

    #[test]
    fn set_input_drives_the_light_register() {
        let mut d = reg_dev();
        d.set_input("lux", 1000.0).unwrap();
        let b = read_reg(&mut d, 0x01, 2);
        let word = (b[0] as u16) | ((b[1] as u16) << 8);
        assert_eq!(word, 1000);
    }

    #[test]
    fn out_of_range_and_unknown_channels_are_rejected() {
        let mut d = reg_dev();
        assert!(d.set_input("lux", -1.0).is_err());
        assert!(d.set_input("nope", 1.0).is_err());
    }

    #[test]
    fn unknown_register_reads_a_zero_word() {
        let mut d = reg_dev();
        let b = read_reg(&mut d, 0x7E, 2);
        assert_eq!(b, vec![0x00, 0x00]);
    }

    // ── command mode: dispatch + CRC-8 exactly matches sensirion ────────────

    #[test]
    fn read_measurement_crc_matches_sensirion_encode_words() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0xEC05);
        let bytes = read_bytes(&mut d, 6);
        // co2 = 450, temperature word = round(22*372.771428 + 16776.75) = 24978.
        let expected = encode_words(&[450, 24978]);
        assert_eq!(bytes, expected, "byte-exact with sensirion framing");
        for chunk in bytes.chunks(3) {
            assert_eq!(chunk[2], sensirion_crc8(&chunk[..2]));
        }
    }

    #[test]
    fn const_response_word_is_served() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0xE4B8); // get_data_ready
        let b = read_bytes(&mut d, 3);
        assert_eq!(b, vec![0x80, 0x06, sensirion_crc8(&[0x80, 0x06])]);
    }

    #[test]
    fn command_source_reflects_set_input() {
        let mut d = cmd_dev();
        d.set_input("co2", 1400.0).unwrap();
        send_cmd(&mut d, 0xEC05);
        let b = read_bytes(&mut d, 3);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 1400);
    }

    #[test]
    fn write_only_command_queues_no_response() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0x21B1); // start_periodic, no response
        let b = read_bytes(&mut d, 3);
        assert!(b.iter().all(|&x| x == 0xFF), "no response bytes: {b:02x?}");
    }

    #[test]
    fn unknown_command_queues_no_response() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0xDEAD);
        let b = read_bytes(&mut d, 3);
        assert!(b.iter().all(|&x| x == 0xFF));
    }

    // ── command mode: params_words accepted and ignored ────────────────────

    #[test]
    fn params_words_are_accepted_and_ignored() {
        let mut d = cmd_dev();
        // set_offset takes 1 parameter word: code then [hi, lo, crc].
        d.start();
        d.write(0x24);
        d.write(0x1D);
        d.write(0x01); // param hi
        d.write(0x2C); // param lo
        d.write(sensirion_crc8(&[0x01, 0x2C])); // param crc
        d.stop();
        // No response queued, and a later command still works.
        let ignored = read_bytes(&mut d, 3);
        assert!(ignored.iter().all(|&x| x == 0xFF));
        send_cmd(&mut d, 0xEC05);
        let b = read_bytes(&mut d, 3);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 450);
    }

    // ── command mode: single-byte opcode dispatch (code_width: 1) ──────────

    #[test]
    fn code_width_1_dispatches_on_first_byte() {
        // cont_hres (0x10) sources lux (default 600) with the datasheet
        // counts-per-lux factor 1.2 ⇒ round(600 * 1.2) = 720, big-endian, no CRC.
        let mut d = cw1_dev();
        assert_eq!(d.address(), 0x23);
        send_byte_cmd(&mut d, 0x10);
        let b = read_bytes(&mut d, 2);
        assert_eq!(
            ((b[0] as u16) << 8) | b[1] as u16,
            720,
            "BE raw = lux * 1.2"
        );
        assert_eq!(b, vec![0x02, 0xD0]);
    }

    #[test]
    fn code_width_1_source_reflects_set_input() {
        let mut d = cw1_dev();
        d.set_input("lux", 1200.0).unwrap();
        send_byte_cmd(&mut d, 0x10);
        let b = read_bytes(&mut d, 2);
        assert_eq!(((b[0] as u16) << 8) | b[1] as u16, 1440);
    }

    #[test]
    fn code_width_1_write_only_opcode_queues_no_response() {
        let mut d = cw1_dev();
        send_byte_cmd(&mut d, 0x01); // power_on, no response
        let b = read_bytes(&mut d, 2);
        assert!(b.iter().all(|&x| x == 0xFF), "no response bytes: {b:02x?}");
    }

    #[test]
    fn code_width_1_unknown_opcode_queues_no_response() {
        let mut d = cw1_dev();
        send_byte_cmd(&mut d, 0xAB);
        let b = read_bytes(&mut d, 2);
        assert!(b.iter().all(|&x| x == 0xFF));
    }

    #[test]
    fn code_width_defaults_to_two() {
        // The command fixture omits code_width ⇒ 16-bit opcode dispatch, so a
        // single written byte must NOT dispatch.
        let mut d = cmd_dev();
        d.start();
        d.write(0xE4); // first byte of get_data_ready (0xE4B8)
        let early = read_bytes(&mut d, 3);
        assert!(early.iter().all(|&x| x == 0xFF), "no dispatch on 1 byte");
    }

    #[test]
    fn invalid_code_width_is_rejected() {
        let yaml = r#"
type: bad_code_width
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x10
    code_width: 3
    commands:
      - { name: c, code: 0x01 }
"#;
        assert!(GenericI2cDevice::from_yaml(yaml, 0).is_err());
    }

    // ── command mode: delay_us data-ready gating ───────────────────────────

    #[test]
    fn delay_us_gates_response_until_time_elapses() {
        let mut d = cmd_dev();
        send_cmd(&mut d, 0x219D); // measure_single_shot, delay 5000 µs
                                  // Before the delay elapses: not ready ⇒ 0xFF.
        let early = read_bytes(&mut d, 3);
        assert!(
            early.iter().all(|&x| x == 0xFF),
            "not ready yet: {early:02x?}"
        );
        // Advance short of the deadline: still not ready.
        d.advance_time_us(4999);
        let still = read_bytes(&mut d, 3);
        assert!(still.iter().all(|&x| x == 0xFF));
        // Cross the deadline: the response materialises.
        d.advance_time_us(1);
        let ready = read_bytes(&mut d, 3);
        assert_eq!(((ready[0] as u16) << 8) | ready[1] as u16, 450);
        assert_eq!(ready[2], sensirion_crc8(&ready[..2]));
    }

    // ── the generic crc8 helper matches the sensirion one ──────────────────

    #[test]
    fn generic_crc8_matches_sensirion_with_default_params() {
        for data in [&[0xBE, 0xEF][..], &[0x01, 0xC2][..], &[0x80, 0x06][..]] {
            assert_eq!(crc8(data, 0x31, 0xFF), sensirion_crc8(data));
        }
    }

    // ── spec validation ────────────────────────────────────────────────────

    #[test]
    fn a_device_declaring_both_shapes_is_rejected() {
        let yaml = r#"
type: bad
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x10
    registers:
      - { name: A, addr: 0, width: 2, endian: le, access: r }
    commands:
      - { name: c, code: 0x0001 }
"#;
        assert!(GenericI2cDevice::from_yaml(yaml, 0).is_err());
    }

    // ── data_ready: write-triggered, time-gated status bits ────────────────
    //
    // Driven against the SHIPPING VCNL4010 descriptor rather than a fixture, so
    // these assert the real part's datasheet numbers (COMMAND 0x80, prox_od
    // 0x08 → prox_data_rdy 0x20 after 570 µs, als_od 0x10 → als_data_rdy 0x40
    // after 100 ms, cleared by a read of the matching result register).

    const COMMAND: u8 = 0x80;
    const PROX_DATA: u8 = 0x87;
    const AMBI_DATA: u8 = 0x85;
    const PROX_RDY: u8 = 0x20;
    const ALS_RDY: u8 = 0x40;

    fn vcnl() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(
            labwired_config::embedded_device_yaml("vcnl4010").expect("embedded"),
            0,
        )
        .expect("vcnl4010.yaml is a valid descriptor")
    }

    /// A VCNL4010 on a bus WITH an honest µs source: one non-zero advance is
    /// what proves the source exists, so nudge the clock before the script.
    fn vcnl_timed() -> GenericI2cDevice {
        let mut d = vcnl();
        d.advance_time_us(1);
        d
    }

    fn write8(d: &mut GenericI2cDevice, reg: u8, value: u8) {
        d.start();
        d.write(reg);
        d.write(value);
        d.stop();
    }

    fn read8(d: &mut GenericI2cDevice, reg: u8) -> u8 {
        read_reg(d, reg, 1)[0]
    }

    #[test]
    fn ready_bit_is_clear_until_the_conversion_time_elapses() {
        let mut d = vcnl_timed();
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            0,
            "nothing has been measured yet, so no result is available"
        );
        write8(&mut d, COMMAND, 0x08); // prox_od
        assert_eq!(read8(&mut d, COMMAND) & PROX_RDY, 0, "conversion in flight");
        d.advance_time_us(569);
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            0,
            "one µs short of the 570 µs conversion time"
        );
        d.advance_time_us(1);
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            PROX_RDY,
            "the conversion time has elapsed: the result is available"
        );
    }

    #[test]
    fn reading_the_result_clears_the_ready_bit() {
        let mut d = vcnl_timed();
        write8(&mut d, COMMAND, 0x08);
        d.advance_time_us(570);
        assert_eq!(read8(&mut d, COMMAND) & PROX_RDY, PROX_RDY);
        // Datasheet: "this bit will be reset when one of the corresponding
        // result registers (reg #7, reg #8) is read".
        let counts = read_reg(&mut d, PROX_DATA, 2);
        assert_eq!(counts, vec![0x07, 0xD0], "default 2000 counts, big-endian");
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            0,
            "the result was consumed, so a fresh conversion is under way"
        );
        // prox_od is still set, so the next conversion re-arms rather than the
        // part going idle — a polling sketch keeps getting fresh readings.
        d.advance_time_us(570);
        assert_eq!(read8(&mut d, COMMAND) & PROX_RDY, PROX_RDY);
    }

    #[test]
    fn firmware_cannot_forge_or_clear_a_ready_bit() {
        let mut d = vcnl_timed();
        // Write every bit: only the low five (write_mask 0x1F) may land, so the
        // data-ready flags stay clear and config_lock stays set.
        write8(&mut d, COMMAND, 0xFF);
        assert_eq!(
            read8(&mut d, COMMAND) & (PROX_RDY | ALS_RDY),
            0,
            "a firmware write must never forge readiness"
        );
        assert_eq!(read8(&mut d, COMMAND) & 0x80, 0x80, "config_lock reads 1");
        assert_eq!(read8(&mut d, COMMAND) & 0x1F, 0x1F, "the enables did land");
        // Once a conversion completes, a write cannot clear the flag either.
        d.advance_time_us(570);
        assert_eq!(read8(&mut d, COMMAND) & PROX_RDY, PROX_RDY);
        write8(&mut d, COMMAND, 0x00);
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            PROX_RDY,
            "only a result read clears it"
        );
    }

    #[test]
    fn ambient_and_proximity_convert_independently() {
        let mut d = vcnl_timed();
        write8(&mut d, COMMAND, 0x18); // als_od | prox_od together
        d.advance_time_us(570);
        let cmd = read8(&mut d, COMMAND);
        assert_eq!(cmd & PROX_RDY, PROX_RDY, "proximity takes 570 µs");
        assert_eq!(cmd & ALS_RDY, 0, "ambient needs the full 100 ms frame");
        d.advance_time_us(100_000 - 570);
        assert_eq!(read8(&mut d, COMMAND) & ALS_RDY, ALS_RDY);
        // Clearing one leaves the other alone.
        read_reg(&mut d, AMBI_DATA, 2);
        let cmd = read8(&mut d, COMMAND);
        assert_eq!(cmd & ALS_RDY, 0, "the ambient result was consumed");
        assert_eq!(cmd & PROX_RDY, PROX_RDY, "the proximity result was not");
    }

    /// The bug this primitive exists to catch: firmware that starts a
    /// conversion and reads the result without waiting for the ready flag.
    /// The twin must report the flag clear, exactly as silicon would.
    #[test]
    fn a_missing_data_ready_poll_is_visible() {
        let mut d = vcnl_timed();
        d.set_input("proximity", 31_000.0).unwrap();
        write8(&mut d, COMMAND, 0x08);
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            0,
            "firmware skipping the poll reads a result that is not ready"
        );
    }

    /// Holdout families (STM32, ESP32-classic, nRF52) never advance the clock,
    /// and the flags must degrade to always-set there — the same always-ready
    /// constant this part modelled before the primitive existed. Anything else
    /// would hang correct firmware inside a vendor poll loop.
    #[test]
    fn without_a_time_source_the_flags_read_always_ready() {
        let mut d = vcnl(); // no advance_time_us — no honest µs source
        assert_eq!(
            read8(&mut d, COMMAND),
            0xE0,
            "config_lock + both ready bits"
        );
        write8(&mut d, COMMAND, 0x08);
        assert_eq!(read8(&mut d, COMMAND), 0xE8, "…plus the enable that landed");
        read_reg(&mut d, PROX_DATA, 2);
        assert_eq!(
            read8(&mut d, COMMAND) & PROX_RDY,
            PROX_RDY,
            "a result read cannot un-ready a device with no clock to wait on"
        );
    }

    #[test]
    fn a_zero_length_advance_does_not_claim_a_time_source() {
        // The central drive runs every scheduler slice and often hands over 0 µs;
        // that proves nothing about whether the chip has an absolute counter.
        let mut d = vcnl();
        d.advance_time_us(0);
        assert_eq!(read8(&mut d, COMMAND), 0xE0);
    }

    // ── data_ready: spec validation ────────────────────────────────────────

    /// A `data_ready` rule with a mistake would silently never fire, which is
    /// the one failure mode that hangs firmware inside a vendor poll loop, so
    /// each is rejected at construction instead.
    #[test]
    fn malformed_data_ready_rules_are_rejected() {
        let build = |rule: &str, regs: &str| {
            let yaml = format!(
                "type: dr_bad\nbehavior:\n  primitive: i2c_device\n  i2c:\n    \
                 default_address: 0x10\n    registers:\n{regs}    data_ready:\n{rule}"
            );
            GenericI2cDevice::from_yaml(&yaml, 0)
        };
        const GOOD_REGS: &str = "      - { name: CMD, addr: 0x00, width: 1, endian: be, access: rw, write_mask: 0x0F }\n      - { name: OUT, addr: 0x01, width: 2, endian: be, access: r }\n";
        const GOOD_RULE: &str = "      - { name: m, start_register: CMD, start_mask: 0x01, ready_register: CMD, ready_mask: 0x10, conversion_us: 100, clear_on_read: [OUT] }\n";
        assert!(build(GOOD_RULE, GOOD_REGS).is_ok(), "the baseline is valid");

        // A register name that does not exist — in any of the three roles.
        for rule in [
            "      - { name: m, start_register: NOPE, start_mask: 0x01, ready_register: CMD, ready_mask: 0x10, conversion_us: 100 }\n",
            "      - { name: m, start_register: CMD, start_mask: 0x01, ready_register: NOPE, ready_mask: 0x10, conversion_us: 100 }\n",
            "      - { name: m, start_register: CMD, start_mask: 0x01, ready_register: CMD, ready_mask: 0x10, conversion_us: 100, clear_on_read: [NOPE] }\n",
        ] {
            assert!(build(rule, GOOD_REGS).is_err(), "unknown register: {rule}");
        }
        // An empty mask could never fire.
        assert!(build(
            "      - { name: m, start_register: CMD, start_mask: 0x00, ready_register: CMD, ready_mask: 0x10, conversion_us: 100 }\n",
            GOOD_REGS
        )
        .is_err());
        // A start bit firmware cannot reach (outside write_mask), and a
        // read-only start register: both mean no conversion can ever start.
        assert!(build(
            "      - { name: m, start_register: CMD, start_mask: 0x10, ready_register: CMD, ready_mask: 0x20, conversion_us: 100 }\n",
            GOOD_REGS
        )
        .is_err());
        assert!(build(
            "      - { name: m, start_register: OUT, start_mask: 0x01, ready_register: CMD, ready_mask: 0x10, conversion_us: 100 }\n",
            GOOD_REGS
        )
        .is_err());
        // A ready bit firmware COULD write would let a sketch forge readiness.
        assert!(build(
            "      - { name: m, start_register: CMD, start_mask: 0x01, ready_register: CMD, ready_mask: 0x02, conversion_us: 100 }\n",
            GOOD_REGS
        )
        .is_err());
        // data_ready needs a register-pointer device to gate a bit in.
        let cmd_mode = "type: dr_cmd\nbehavior:\n  primitive: i2c_device\n  i2c:\n    default_address: 0x10\n    commands:\n      - { name: c, code: 0x01 }\n    data_ready:\n      - { name: m, start_register: CMD, start_mask: 0x01, ready_register: CMD, ready_mask: 0x10, conversion_us: 100 }\n";
        assert!(GenericI2cDevice::from_yaml(cmd_mode, 0).is_err());
    }

    // ── data_ready is not VCNL4010-shaped ──────────────────────────────────

    /// The VCNL4010 puts the start bit and the ready bit in the SAME register.
    /// If the primitive only worked for that it would be a device feature with
    /// a schema, not a primitive. This fixture is the OTHER common shape, taken
    /// from the ST VL53L0X: the start bit lives in `SYSRANGE_START` (0x00), the
    /// ready bit in a different register `RESULT_INTERRUPT_STATUS` (0x13), and
    /// it is cleared by reading a third, `RESULT_RANGE_VAL` (0x1E). Adopting
    /// the primitive for it is YAML only — no Rust, no schema change.
    ///
    /// (The real VL53L0X has SINCE been migrated: `configs/devices/vl53l0x.yaml`
    /// is the shipping model and the hand-written `components/vl53l0x.rs` is
    /// deleted. It needed two more engine capabilities this fixture does not
    /// exercise — `auto_increment` for the 12-byte block read at 0x14, and
    /// `clear_on_write` for `SYSTEM_INTERRUPT_CLEAR`, since reading the range
    /// does NOT acknowledge on that part. See `tests/vl53l0x_migration_parity.rs`.
    /// This fixture stays as the minimal split-register shape.)
    const TOF_FIXTURE: &str = r#"
type: test_tof_data_ready_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x29
    registers:
      - { name: SYSRANGE_START, addr: 0x00, width: 1, endian: be, access: rw, reset: 0x00 }
      - { name: RESULT_INTERRUPT_STATUS, addr: 0x13, width: 1, endian: be, access: r, reset: 0x00 }
      - { name: RESULT_RANGE_VAL, addr: 0x1E, width: 2, endian: be, access: r, source: distance }
      - { name: MODEL_ID, addr: 0xC0, width: 1, endian: be, access: r, reset: 0xEE }
    data_ready:
      - name: range
        start_register: SYSRANGE_START
        start_mask: 0x01
        ready_register: RESULT_INTERRUPT_STATUS
        ready_mask: 0x07
        conversion_us: 33000
        clear_on_read: [RESULT_RANGE_VAL]
metadata:
  inputs:
    - { key: distance, label: "Distance", unit: mm, min: 0, max: 2000, default: 200 }
"#;

    #[test]
    fn a_second_device_shape_adopts_data_ready_in_yaml_only() {
        let mut d = GenericI2cDevice::from_yaml(TOF_FIXTURE, 0).unwrap();
        d.advance_time_us(1); // honest µs source present
        assert_eq!(read8(&mut d, 0xC0), 0xEE, "identification is untouched");
        assert_eq!(
            read8(&mut d, 0x13),
            0x00,
            "no ranging started ⇒ the interrupt status is clear"
        );
        write8(&mut d, 0x00, 0x01); // SYSRANGE_START
        assert_eq!(read8(&mut d, 0x13), 0x00, "measuring");
        d.advance_time_us(32_999);
        assert_eq!(read8(&mut d, 0x13), 0x00, "one µs short of the budget");
        d.advance_time_us(1);
        assert_eq!(read8(&mut d, 0x13), 0x07, "the range is ready");
        // The flag lives in a different register from the start bit, and is
        // cleared by reading a third.
        assert_eq!(read_reg(&mut d, 0x1E, 2), vec![0x00, 0xC8], "200 mm");
        assert_eq!(
            read8(&mut d, 0x13),
            0x00,
            "reading the range consumed the result"
        );
        // Still in continuous mode (the start bit is set), so it re-arms.
        d.advance_time_us(33_000);
        assert_eq!(read8(&mut d, 0x13), 0x07);
    }

    #[test]
    fn write_mask_protects_bits_outside_it() {
        // Independent of data_ready: an rw register with a write_mask keeps the
        // bits silicon owns, and an absent mask still replaces the whole word.
        let yaml = "type: wm\nbehavior:\n  primitive: i2c_device\n  i2c:\n    \
             default_address: 0x10\n    registers:\n      \
             - { name: A, addr: 0x00, width: 1, endian: be, access: rw, write_mask: 0x0F, reset: 0xA0 }\n      \
             - { name: B, addr: 0x01, width: 1, endian: be, access: rw, reset: 0xA0 }\n";
        let mut d = GenericI2cDevice::from_yaml(yaml, 0).unwrap();
        write8(&mut d, 0x00, 0xFF);
        assert_eq!(
            read8(&mut d, 0x00),
            0xAF,
            "high nibble kept, low nibble set"
        );
        write8(&mut d, 0x01, 0xFF);
        assert_eq!(read8(&mut d, 0x01), 0xFF, "no mask ⇒ full replacement");
    }

    // ─── Tier 1: register side effects (`on_read` / `on_write`) ────────────

    /// A fictional interrupt-status part: one write-1-to-clear register, one
    /// write-0-to-clear, one one-to-set, and a clear-on-read status — the four
    /// SystemRDL actions on one device, each spelled with a DIFFERENT alias so
    /// the alias table is exercised rather than assumed.
    const SIDE_EFFECT_FIXTURE: &str = r#"
type: test_i2c_side_effect_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x40
    registers:
      - { name: INT_STATUS, addr: 0x00, width: 1, endian: be, access: rw, reset: 0xF0, on_write: one_to_clear }
      - { name: INT_LATCH, addr: 0x01, width: 1, endian: be, access: rw, reset: 0xFF, on_write: write_zero_to_clear }
      - { name: INT_ENABLE, addr: 0x02, width: 1, endian: be, access: rw, reset: 0x00, on_write: oneToSet }
      - { name: FAULT, addr: 0x03, width: 1, endian: be, access: rw, reset: 0x3C, on_read: clear }
      - { name: MASKED_W1C, addr: 0x04, width: 1, endian: be, access: rw, reset: 0xFF, write_mask: 0x0F, on_write: w1c }
      - { name: WIDE_W1C, addr: 0x05, width: 2, endian: be, access: rw, reset: 0xFFFF, on_write: one_to_clear }
"#;

    /// The error text of a descriptor that must fail to load. `expect_err`
    /// would need `Debug` on the device, which it deliberately does not have.
    fn err_of(result: Result<GenericI2cDevice>, what: &str) -> String {
        match result {
            Ok(_) => panic!("{what} must be rejected, but it loaded"),
            Err(e) => e.to_string(),
        }
    }

    fn side_effects() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(SIDE_EFFECT_FIXTURE, 0).unwrap()
    }

    /// The same map with byte-wise pointer auto-increment, which reaches a
    /// different store path (`write_byte_at`) and a different read path.
    fn side_effects_ai() -> GenericI2cDevice {
        let yaml = SIDE_EFFECT_FIXTURE.replace(
            "    default_address: 0x40",
            "    default_address: 0x40\n    auto_increment: true",
        );
        GenericI2cDevice::from_yaml(&yaml, 0).unwrap()
    }

    #[test]
    fn one_to_clear_clears_the_bits_written_as_one() {
        for (label, mut d) in [("pointed", side_effects()), ("burst", side_effects_ai())] {
            assert_eq!(read8(&mut d, 0x00), 0xF0, "{label}: reset");
            write8(&mut d, 0x00, 0x30);
            assert_eq!(read8(&mut d, 0x00), 0xC0, "{label}: 0xF0 & !0x30");
            // A 0 is inert — the acknowledge idiom only ever clears.
            write8(&mut d, 0x00, 0x00);
            assert_eq!(
                read8(&mut d, 0x00),
                0xC0,
                "{label}: writing 0 changes nothing"
            );
        }
    }

    #[test]
    fn zero_to_clear_clears_the_bits_written_as_zero() {
        for (label, mut d) in [("pointed", side_effects()), ("burst", side_effects_ai())] {
            write8(&mut d, 0x01, 0x0F);
            assert_eq!(read8(&mut d, 0x01), 0x0F, "{label}: 0xFF & 0x0F");
        }
    }

    #[test]
    fn one_to_set_sets_the_bits_written_as_one() {
        for (label, mut d) in [("pointed", side_effects()), ("burst", side_effects_ai())] {
            write8(&mut d, 0x02, 0x01);
            write8(&mut d, 0x02, 0x80);
            assert_eq!(read8(&mut d, 0x02), 0x81, "{label}: writes accumulate");
        }
    }

    #[test]
    fn a_write_action_cannot_reach_bits_outside_the_write_mask() {
        // `write_mask` says which bits firmware owns; `on_write` says what
        // touching them does. A w1c write of 0xFF must clear only the low
        // nibble the mask exposes.
        for (label, mut d) in [("pointed", side_effects()), ("burst", side_effects_ai())] {
            write8(&mut d, 0x04, 0xFF);
            assert_eq!(
                read8(&mut d, 0x04),
                0xF0,
                "{label}: high nibble is silicon's"
            );
        }
    }

    #[test]
    fn a_burst_write_one_to_clear_touches_only_the_byte_it_carries() {
        // The byte-wise path delivers one lane at a time. Clearing with the
        // MERGED word would let the high byte's existing bits clear themselves
        // the moment the low byte arrives.
        let mut d = side_effects_ai();
        d.start();
        d.write(0x05);
        d.write(0x0F); // high byte: clears bits 11:8
        d.stop();
        assert_eq!(read_reg(&mut d, 0x05, 2), vec![0xF0, 0xFF]);
    }

    #[test]
    fn on_read_clear_zeroes_the_register_after_the_read() {
        // The master receives the pre-clear value; the next read sees zero.
        for (label, mut d) in [("pointed", side_effects()), ("burst", side_effects_ai())] {
            assert_eq!(read8(&mut d, 0x03), 0x3C, "{label}: first read");
            assert_eq!(read8(&mut d, 0x03), 0x00, "{label}: cleared by the read");
        }
    }

    #[test]
    fn a_register_without_on_read_is_not_cleared_by_reading_it() {
        // The negative control: without the key the value stays put, which is
        // what every descriptor written before this field meant.
        let mut d = side_effects();
        assert_eq!(read8(&mut d, 0x00), 0xF0);
        assert_eq!(read8(&mut d, 0x00), 0xF0);
    }

    #[test]
    fn every_spelling_of_one_to_clear_is_the_same_value() {
        // `one_to_clear`, `write_one_to_clear` and `oneToClear` are aliases of
        // ONE enum value. If they ever became separate variants a datasheet
        // spelled one way would silently do nothing.
        for spelling in ["one_to_clear", "write_one_to_clear", "oneToClear", "w1c"] {
            let yaml = SIDE_EFFECT_FIXTURE
                .replace("on_write: one_to_clear", &format!("on_write: {spelling}"));
            let mut d = GenericI2cDevice::from_yaml(&yaml, 0)
                .unwrap_or_else(|e| panic!("{spelling} must parse: {e}"));
            write8(&mut d, 0x00, 0x30);
            assert_eq!(read8(&mut d, 0x00), 0xC0, "{spelling}");
        }
    }

    // ─── Tier 1: device timers ─────────────────────────────────────────────

    /// A part with its own clock: a free-running sample timer that raises a
    /// data-ready bit, and a one-shot conversion armed by a start bit.
    const TIMER_FIXTURE: &str = r#"
type: test_i2c_timer_fixture
behavior:
  primitive: i2c_device
  timers:
    - name: sample
      period_us: 1000
      start: on_reset
      on_fire:
        - set_bits: { register: STATUS, bits: 0x01 }
    - name: conversion
      after_us: 5000
      start: manual
      start_on_write: { register: CTRL, mask: 0x01 }
      on_fire:
        - set_bits: { register: STATUS, bits: 0x80 }
        - write_value: { register: RESULT, value: 0x2A }
  i2c:
    default_address: 0x41
    registers:
      - { name: STATUS, addr: 0x00, width: 1, endian: be, access: r, reset: 0x00 }
      - { name: CTRL, addr: 0x01, width: 1, endian: be, access: rw, reset: 0x00 }
      - { name: RESULT, addr: 0x02, width: 1, endian: be, access: r, reset: 0x00 }
"#;

    fn timer_dev() -> GenericI2cDevice {
        GenericI2cDevice::from_yaml(TIMER_FIXTURE, 0).unwrap()
    }

    #[test]
    fn a_periodic_timer_does_not_fire_before_its_period() {
        let mut d = timer_dev();
        assert_eq!(read8(&mut d, 0x00), 0x00, "nothing has elapsed");
        d.advance_time_us(999);
        assert_eq!(read8(&mut d, 0x00), 0x00, "one µs short");
        d.advance_time_us(1);
        assert_eq!(read8(&mut d, 0x00), 0x01, "the sample timer fired");
    }

    #[test]
    fn a_manual_timer_stays_idle_until_its_start_write() {
        let mut d = timer_dev();
        d.advance_time_us(1_000_000);
        assert_eq!(read8(&mut d, 0x02), 0x00, "no conversion was ever started");
        assert_eq!(read8(&mut d, 0x00) & 0x80, 0x00);
    }

    #[test]
    fn a_one_shot_timer_fires_once_after_its_delay() {
        let mut d = timer_dev();
        write8(&mut d, 0x01, 0x01); // CTRL start bit
        d.advance_time_us(4_999);
        assert_eq!(read8(&mut d, 0x02), 0x00, "the conversion is not done");
        d.advance_time_us(1);
        assert_eq!(read8(&mut d, 0x02), 0x2A, "write_value landed");
        assert_eq!(read8(&mut d, 0x00) & 0x80, 0x80, "set_bits landed");
        // One-shot: it does not come back on its own.
        d.reg_values.insert("RESULT".into(), 0);
        d.advance_time_us(50_000);
        assert_eq!(read8(&mut d, 0x02), 0x00, "a one-shot must not repeat");
    }

    #[test]
    fn a_start_write_that_leaves_the_mask_clear_does_not_start_the_timer() {
        let mut d = timer_dev();
        write8(&mut d, 0x01, 0x02); // a bit outside start_on_write's mask
        d.advance_time_us(100_000);
        assert_eq!(read8(&mut d, 0x02), 0x00, "the mask is level-triggered");
    }

    #[test]
    fn one_advance_replays_every_period_it_covers() {
        // A late service pass must see the samples that accrued while the CPU
        // was elsewhere, not one merged tick.
        let mut d = timer_dev();
        write8(&mut d, 0x01, 0x01);
        d.advance_time_us(10_000);
        // The one-shot fired at 5 000 and the periodic ten times; both actions
        // are visible in one pass.
        assert_eq!(read8(&mut d, 0x00), 0x81);
        assert_eq!(read8(&mut d, 0x02), 0x2A);
    }

    #[test]
    fn a_device_with_no_timers_is_untouched_by_time() {
        // The short-circuit that keeps every shipped device byte-identical.
        let mut d = reg_dev();
        d.advance_time_us(10_000_000);
        assert_eq!(read_reg(&mut d, 0x00, 2), read_reg(&mut d, 0x00, 2));
    }

    #[test]
    fn malformed_timers_are_rejected_at_load() {
        let cases: &[(&str, &str)] = &[
            (
                "      - { name: t, period_us: 1000, after_us: 10, on_fire: [ { set_bits: { register: STATUS, bits: 1 } } ] }\n",
                "both period_us and after_us",
            ),
            (
                "      - { name: t, on_fire: [ { set_bits: { register: STATUS, bits: 1 } } ] }\n",
                "neither period_us nor after_us",
            ),
            (
                "      - { name: t, period_us: 0, on_fire: [ { set_bits: { register: STATUS, bits: 1 } } ] }\n",
                "zero interval",
            ),
            (
                "      - { name: t, period_us: 10, on_fire: [] }\n",
                "no on_fire actions",
            ),
            (
                "      - { name: t, period_us: 10, on_fire: [ { set_bits: { register: NOPE, bits: 1 } } ] }\n",
                "not a declared register",
            ),
            (
                "      - { name: t, period_us: 10, start_on_write: { register: NOPE }, on_fire: [ { set_bits: { register: STATUS, bits: 1 } } ] }\n",
                "not a declared register",
            ),
        ];
        for (timer, expected) in cases {
            let yaml = format!(
                "type: t\nbehavior:\n  primitive: i2c_device\n  timers:\n{timer}  i2c:\n    default_address: 0x41\n    registers:\n      - {{ name: STATUS, addr: 0x00, width: 1, endian: be, access: r, reset: 0x00 }}\n"
            );
            let err = err_of(GenericI2cDevice::from_yaml(&yaml, 0), "malformed timer");
            assert!(err.contains(expected), "got: {err}\nwanted: {expected}");
        }
    }

    #[test]
    fn a_timer_needs_a_named_register_map() {
        // A command device or a register file has no register names, so a timer
        // there could never do anything. Say so at load.
        let yaml = "type: t\nbehavior:\n  primitive: i2c_device\n  timers:\n    - { name: t, period_us: 10, on_fire: [ { set_bits: { register: STATUS, bits: 1 } } ] }\n  i2c:\n    default_address: 0x41\n    register_file: { size: 8 }\n";
        let err = err_of(
            GenericI2cDevice::from_yaml(yaml, 0),
            "timer without a register map",
        );
        assert!(err.contains("needs a named register map"), "got: {err}");
    }

    // ─── Tier 1: 16-bit pointers and paged writes ──────────────────────────

    const EEPROM_FIXTURE: &str = r#"
type: test_i2c_eeprom_fixture
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x50
    pointer_width: 2
    write_page: 8
    register_file:
      size: 64
      fill: 0xFF
      pointer_mask: 0xFFFF
      auto_increment: always
"#;

    #[test]
    fn a_two_byte_pointer_takes_the_high_byte_first() {
        let mut d = GenericI2cDevice::from_yaml(EEPROM_FIXTURE, 0).unwrap();
        d.start();
        d.write(0x00);
        d.write(0x05);
        d.write(0xAB);
        d.stop();
        d.start();
        d.write(0x00);
        d.write(0x05);
        d.start();
        assert_eq!(d.read(), 0xAB);
    }

    #[test]
    fn an_unwritten_cell_reads_the_declared_fill() {
        let mut d = GenericI2cDevice::from_yaml(EEPROM_FIXTURE, 0).unwrap();
        d.start();
        d.write(0x00);
        d.write(0x00);
        d.start();
        assert_eq!((0..4).map(|_| d.read()).collect::<Vec<_>>(), vec![0xFF; 4]);
    }

    #[test]
    fn a_sequential_write_wraps_inside_its_page() {
        // Four bytes from 0x06 in an 8-byte page: 0x06, 0x07, then wrap to
        // 0x00, 0x01 — NOT 0x08, 0x09.
        let mut d = GenericI2cDevice::from_yaml(EEPROM_FIXTURE, 0).unwrap();
        d.start();
        d.write(0x00);
        d.write(0x06);
        for b in [1u8, 2, 3, 4] {
            d.write(b);
        }
        d.stop();
        d.start();
        d.write(0x00);
        d.write(0x00);
        d.start();
        let page: Vec<u8> = (0..10).map(|_| d.read()).collect();
        assert_eq!(page[0], 3, "the write wrapped to the start of the page");
        assert_eq!(page[1], 4);
        assert_eq!(page[6], 1);
        assert_eq!(page[7], 2);
        assert_eq!(&page[8..10], &[0xFF, 0xFF], "the next page is untouched");
    }

    #[test]
    fn a_sequential_read_is_not_paged() {
        // The datasheets page WRITES only; a read rolls over the whole array.
        let mut d = GenericI2cDevice::from_yaml(EEPROM_FIXTURE, 0).unwrap();
        d.start();
        d.write(0x00);
        d.write(0x06);
        d.start();
        let out: Vec<u8> = (0..4).map(|_| d.read()).collect();
        assert_eq!(out, vec![0xFF; 4]);
        assert_eq!(d.file_pointer, 0x0A, "the read pointer ran past the page");
    }

    #[test]
    fn a_sixteen_bit_register_address_needs_the_wider_pointer() {
        // A named register above 0xFF with a one-byte pointer could never be
        // selected — rejected at load rather than answering nothing.
        let yaml = "type: t\nbehavior:\n  primitive: i2c_device\n  i2c:\n    default_address: 0x41\n    registers:\n      - { name: FAR, addr: 0x0120, width: 1, endian: be, access: r, reset: 0x01 }\n";
        let err = err_of(
            GenericI2cDevice::from_yaml(yaml, 0),
            "far register, one-byte pointer",
        );
        assert!(err.contains("needs pointer_width: 2"), "got: {err}");
    }

    #[test]
    fn a_named_register_map_can_use_a_two_byte_pointer() {
        let yaml = "type: t\nbehavior:\n  primitive: i2c_device\n  i2c:\n    default_address: 0x41\n    pointer_width: 2\n    registers:\n      - { name: NEAR, addr: 0x0002, width: 1, endian: be, access: r, reset: 0x11 }\n      - { name: FAR, addr: 0x0120, width: 1, endian: be, access: r, reset: 0x22 }\n";
        let mut d = GenericI2cDevice::from_yaml(yaml, 0).unwrap();
        d.start();
        d.write(0x01);
        d.write(0x20);
        d.start();
        assert_eq!(d.read(), 0x22, "the high address byte must select FAR");
        d.stop();
        d.start();
        d.write(0x00);
        d.write(0x02);
        d.start();
        assert_eq!(d.read(), 0x11);
    }

    #[test]
    fn an_unsupported_pointer_width_is_rejected() {
        let yaml = "type: t\nbehavior:\n  primitive: i2c_device\n  i2c:\n    default_address: 0x41\n    pointer_width: 3\n    registers:\n      - { name: A, addr: 0x00, width: 1, endian: be, access: r, reset: 0x01 }\n";
        let err = err_of(GenericI2cDevice::from_yaml(yaml, 0), "pointer_width 3");
        assert!(err.contains("pointer_width 3 unsupported"), "got: {err}");
    }

    /// `stream: true` — a port the auto-increment pointer does not walk past,
    /// in BOTH directions. Proved on a minimal fixture as well as on
    /// `bmi270.yaml`, because a load rule only one shipped file exercises is a
    /// rule nobody has checked.
    #[test]
    fn a_stream_port_holds_the_auto_increment_pointer() {
        let yaml = r#"
type: t
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x41
    auto_increment: true
    registers:
      - { name: PORT, addr: 0x00, width: 1, endian: be, access: rw, reset: 0x00, stream: true }
      - { name: NEXT, addr: 0x01, width: 1, endian: be, access: rw, reset: 0xEE }
"#;
        let mut d = GenericI2cDevice::from_yaml(yaml, 0).unwrap();
        // Four bytes into the port: NEXT must be untouched, not overwritten by
        // the walk a stepping pointer would take.
        d.start();
        d.write(0x00);
        for b in [0x11u8, 0x22, 0x33, 0x44] {
            d.write(b);
        }
        d.stop();
        d.start();
        d.write(0x01);
        d.start();
        assert_eq!(d.read(), 0xEE, "the burst must not have walked into NEXT");
        d.stop();
        // And a READ of the port serves its own byte for as long as the master
        // clocks: the last write landed, and the cursor never moved.
        d.start();
        d.write(0x00);
        d.start();
        assert_eq!(
            [d.read(), d.read(), d.read()],
            [0x44, 0x44, 0x44],
            "a port is read the same way it is written"
        );
    }

    /// Without the key the SAME burst walks the map — the failure mode the
    /// BMI270's config upload hits, reduced to two registers.
    #[test]
    fn without_stream_the_same_burst_walks_into_the_next_register() {
        let yaml = r#"
type: t
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x41
    auto_increment: true
    registers:
      - { name: PORT, addr: 0x00, width: 1, endian: be, access: rw, reset: 0x00 }
      - { name: NEXT, addr: 0x01, width: 1, endian: be, access: rw, reset: 0xEE }
"#;
        let mut d = GenericI2cDevice::from_yaml(yaml, 0).unwrap();
        d.start();
        d.write(0x00);
        d.write(0x11);
        d.write(0x22);
        d.stop();
        d.start();
        d.write(0x01);
        d.start();
        assert_eq!(d.read(), 0x22, "the second byte landed on NEXT");
    }

    #[test]
    fn declarative_kit_builds_metadata_from_descriptor() {
        let kit = DeclarativeI2cKit::from_yaml(REGISTER_FIXTURE).unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "test_i2c_fixture");
        assert_eq!(m.inputs.len(), 2);
        assert!(m.inputs.iter().any(|c| c.key == "lux"));
    }
}
