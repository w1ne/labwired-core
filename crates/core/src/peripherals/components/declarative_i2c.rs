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
    AddWrap, AutoIncrement, Crc8Spec, DataReady, DeviceDescriptor, Endian, I2cAccess, I2cCommand,
    I2cRegister, I2cSpec, IndexedTable, ObservableSpec, ReadComplete, ResponseWord, UpdateRule,
};

use super::declarative_regs::{encode_raw, observe, pack, register_read_bytes, unpack};
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

    /// Selected register pointer for the current transaction.
    pointer: Option<u8>,
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
    page_register: Option<u8>,

    /// Indexed readout ports and their per-port fetch state (parallel Vecs).
    /// Empty ⇒ every indexed-table code path short-circuits.
    indexed_tables: Vec<IndexedTable>,
    it_state: Vec<DataReadyState>,

    /// Register-pointer-mode pointer mask (applied to the pointer byte). 0xFF ⇒
    /// no masking (the default; TMP102 uses 0x03).
    reg_pointer_mask: u8,
    /// Register-pointer-mode self-driving update rules (e.g. TMP102 drift).
    updates: Vec<UpdateRule>,
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
    file_pointer: u8,
    file_writes_since_frame: u32,
    file_pointer_mask: u8,
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
}

impl GenericI2cDevice {
    /// Build from a descriptor and pre-leaked channel table.
    pub fn from_descriptor(
        descriptor: &DeviceDescriptor,
        address: u8,
        channels: &'static [InputChannel],
    ) -> Result<Self> {
        let spec = descriptor
            .behavior
            .i2c
            .as_ref()
            .context("declarative i2c device is missing behavior.i2c")?;
        validate_spec(spec)?;

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

        // Byte-addressable register-file mode: allocate the file and stamp resets.
        let file = spec.register_file.as_ref().map(|rf| {
            let mut regs = vec![0u8; rf.size];
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

        Ok(Self {
            address,
            registers: spec.registers.clone(),
            commands: spec.commands.clone(),
            crc8: spec.crc8,
            command_mode: !spec.commands.is_empty(),
            code_width: spec.code_width as usize,
            slots,
            reg_values,
            pointer: None,
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
            reg_pointer_mask: spec.pointer_mask.unwrap_or(0xFF),
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
        })
    }

    /// The slot view a read observes: seeded noise applied to the channels that
    /// declare it — one sample per channel per call, so a register word is a
    /// single observation, matching how firmware experiences a noisy sensor.
    /// Thermal lag uses the same accumulated µs source as `delay_us` gating;
    /// buses without an honest µs source get noise+bias but no lag.
    fn observed_slots(&mut self) -> HashMap<String, f64> {
        if self.noise.is_empty() {
            return self.slots.clone();
        }
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
    }

    /// Read a named observable channel in engineering units (e.g. the PCA9685
    /// `servo_angle` for a channel). Mirrors `IrCore::observable`; only
    /// register-file devices declare observables, so this returns `None` for
    /// register-pointer / command devices.
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
    fn trigger_matches(&self, rc: &ReadComplete, pointer: u8) -> bool {
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
    fn apply_read_complete_updates(&mut self, pointer: u8) {
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
    fn write_byte_at(&mut self, addr: u8, data: u8) {
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
        let (name, endian, width, write_mask, self_clearing) = (
            reg.name.clone(),
            reg.endian,
            reg.width,
            reg.write_mask,
            reg.self_clearing,
        );
        let idx = usize::from(addr - reg.addr);
        let prev = self.reg_values.get(&name).copied().unwrap_or(0);
        // Place the byte at its position in the word, honouring the declared
        // byte order, so a byte-wise burst reassembles the same word a
        // width-sized write would have stored.
        let shift = 8 * match endian {
            Endian::Be => u32::from(width) - 1 - idx as u32,
            Endian::Le => idx as u32,
        };
        let written = (prev & !(0xFFu32 << shift)) | (u32::from(data) << shift);
        // `write_mask` protects the bits silicon owns; see the non-incrementing
        // path above.
        let stored = match write_mask {
            Some(mask) => (prev & !mask) | (written & mask),
            None => written,
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
        // A momentary "go" bit is gone by the time firmware can read it back:
        // the device has already acted on it (see `RegisterSpec::self_clearing`).
        if let Some(mask) = self_clearing {
            if stored & mask != 0 {
                self.reg_values.insert(name, stored & !mask);
            }
        }
    }

    /// Convenience for tests / standalone use: parse a descriptor YAML and leak
    /// its channel table. (The kit path shares one leaked table across attaches;
    /// this leaks per call, which is fine for the few devices a test builds.)
    pub fn from_yaml(yaml: &str, address: u8) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        let channels = leak_channels(&descriptor);
        Self::from_descriptor(&descriptor, address, channels)
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

    /// The register at `addr` in the current bank. A bank-specific register wins
    /// over a bank-agnostic one at the same pointer, so a part can carry a flat
    /// core map plus a handful of aliased addresses.
    fn find_register(&self, addr: u8) -> Option<&I2cRegister> {
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
    fn register_covering(&self, addr: u8) -> Option<&I2cRegister> {
        let covers = |r: &&I2cRegister| {
            addr >= r.addr && u16::from(addr) < u16::from(r.addr) + u16::from(r.width)
        };
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
    fn byte_at(&self, addr: u8, slots: &HashMap<String, f64>) -> (u8, Option<(String, u8)>) {
        let Some(reg) = self.register_covering(addr) else {
            return (self.reg_unmapped_byte, None);
        };
        let raw = register_read_bytes(reg, slots, &self.reg_values);
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
    /// `slots` is the noise-applied observation view computed by the caller.
    fn build_response(&self, cmd: &I2cCommand, slots: &HashMap<String, f64>) -> Vec<u8> {
        let mut out = Vec::new();
        for word in &cmd.response {
            let raw = Self::response_word_raw(word, slots);
            let bytes = pack(raw, word.width, Endian::Be); // commands are BE on wire
            match &self.crc8 {
                // CRC framing is per 16-bit word, exactly like the Sensirion
                // read buffer (see super::sensirion::encode_words).
                Some(c) => {
                    for chunk in bytes.chunks(2) {
                        out.extend_from_slice(chunk);
                        out.push(crc8(chunk, c.poly, c.init));
                    }
                }
                None => out.extend_from_slice(&bytes),
            }
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
        let resp = self.build_response(&cmd, &slots);
        match cmd.delay_us {
            Some(us) if us > 0 => {
                self.pending = Some(resp);
                self.ready_at_us = self.elapsed_us + us;
            }
            _ => self.read_buf = resp,
        }
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
    }

    fn stop(&mut self) {
        // End of transaction: clear the write accumulator so the next command /
        // pointer starts fresh (the C3 controller only calls start() on a
        // repeated START, so the real reset happens here — same as veml7700 /
        // scd41).
        self.write_buf.clear();
    }

    fn write(&mut self, data: u8) {
        // Register-file mode (byte-addressable): first post-START byte selects
        // the pointer; subsequent bytes are data, and the pointer auto-increments
        // when its enable field is set. The enable is checked LIVE (after the
        // store) so the write that sets the field also advances the pointer.
        if let Some(regs) = self.file.as_mut() {
            if self.file_first_write_sets_pointer && self.file_writes_since_frame == 0 {
                self.file_pointer = data & self.file_pointer_mask;
            } else if !regs.is_empty() {
                let idx = self.file_pointer as usize % regs.len();
                regs[idx] = data;
                if Self::file_ai_enabled(&self.file_auto_increment, regs) {
                    self.file_pointer = self.file_pointer.wrapping_add(1);
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
        // Register mode: first byte is the pointer (masked); the rest are a data
        // write into the pointed rw register.
        if self.write_buf.len() == 1 {
            self.pointer = Some(data & self.reg_pointer_mask);
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
            self.pointer = Some(ptr.wrapping_add(1));
            self.write_byte_at(ptr, data);
            return;
        }
        let Some(reg) = self.find_register(ptr) else {
            return;
        };
        if reg.access != I2cAccess::Rw || self.write_buf.len() != 1 + reg.width as usize {
            return;
        }
        let (name, endian, write_mask) = (reg.name.clone(), reg.endian, reg.write_mask);
        let written = unpack(&self.write_buf[1..], endian);
        // `write_mask` protects the bits silicon owns (a model-driven status
        // flag, a hardwired bit): those keep their current value. Absent ⇒ the
        // whole word is replaced, exactly as before the mask existed.
        let stored = match write_mask {
            Some(mask) => {
                let prev = self.reg_values.get(&name).copied().unwrap_or(0);
                (prev & !mask) | (written & mask)
            }
            None => written,
        };
        self.reg_values.insert(name.clone(), stored);
        if !self.data_ready.is_empty() {
            // Acknowledge first, then start: a part whose start and clear
            // registers are the same would otherwise clear the conversion it
            // just started. Ordering it this way makes "write 1 to clear, then
            // write 1 to start" behave like the two operations it is.
            self.clear_on_write(&name);
            self.start_conversions(&name, stored);
        }
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
            self.pointer = Some(addr.wrapping_add(1));
            // Clear only once the whole word has been delivered: clearing on the
            // first byte of a 2-byte result would drop the flag while the master
            // is still mid-read. Same reason the self-driving updates fire here
            // and are keyed on the register's START address, which is what
            // `apply_read_complete_updates` matches a trigger against.
            if let Some((name, start)) = hit {
                if !self.data_ready.is_empty() {
                    self.clear_on_read(&name);
                }
                if !self.updates.is_empty() {
                    self.apply_read_complete_updates(start);
                }
                // Word complete: the next word is a new observation.
                self.observed = None;
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
                    let raw = register_read_bytes(reg, &slots, &self.reg_values);
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
            self.read_buf = bytes;
            // The datasheets clear the flag on a read of the result register
            // ("reset when one of the corresponding result registers is read"),
            // so it happens as the read latches, not after the last byte.
            if let Some(name) = name {
                if !self.data_ready.is_empty() {
                    self.clear_on_read(&name);
                }
            }
            self.latched = true;
        }
        let byte = self.read_buf.get(self.read_idx).copied().unwrap_or(0xFF);
        self.read_idx += 1;
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

/// A descriptor is exactly one shape (registers XOR commands XOR register_file),
/// and command devices with CRC framing must use even-width words (CRC is
/// computed per 16-bit word).
fn validate_spec(spec: &I2cSpec) -> Result<()> {
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
    for reg in spec.registers.iter().filter(|r| r.zero_when.is_some()) {
        let z = reg.zero_when.as_ref().expect("filtered");
        let gate = match spec.registers.iter().find(|r| r.name == z.register) {
            Some(g) => g,
            None => bail!(
                "register '{}' zero_when register '{}' is not a declared register",
                reg.name,
                z.register
            ),
        };
        if z.mask == 0 {
            bail!(
                "register '{}' zero_when mask is 0 — the gate could never fire",
                reg.name
            );
        }
        if z.register == reg.name {
            bail!(
                "register '{}' zero_when gates itself — the gate would erase the very bits \
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
                "register '{}' zero_when mask {:#x} includes bits firmware cannot write in '{}' \
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
    if spec.crc8.is_some() {
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
fn config_type_from_str(ty: &str) -> ConfigType {
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
        // Honour `config:` overrides that name an input channel (e.g. a `lux`
        // seed), matching how a hand-written kit seeded its initial reading.
        for input in self.channels {
            if let Some(v) = ctx.config_f64(input.key) {
                device.seed_input(input.key, v);
            }
        }
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

    #[test]
    fn declarative_kit_builds_metadata_from_descriptor() {
        let kit = DeclarativeI2cKit::from_yaml(REGISTER_FIXTURE).unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "test_i2c_fixture");
        assert_eq!(m.inputs.len(), 2);
        assert!(m.inputs.iter().any(|c| c.key == "lux"));
    }
}
