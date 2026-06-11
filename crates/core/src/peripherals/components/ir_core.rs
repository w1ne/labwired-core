// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bus-agnostic behavioral engine for declarative [`IrComponent`] specs.
//!
//! `IrCore` is the single source of truth for *what a device does*; it knows
//! nothing about *which bus* carries the bytes. It exposes three bus-neutral
//! primitives:
//!
//! - [`reset_frame`](IrCore::reset_frame) — a transaction boundary (I2C START,
//!   SPI CS edge, UART break, …);
//! - [`write`](IrCore::write) — one master→device byte;
//! - [`read`](IrCore::read) — one device→master byte.
//!
//! Per-bus adapters ([`IrI2cComponent`](super::ir_component::IrI2cComponent),
//! [`IrSpiComponent`](super::ir_spi_component::IrSpiComponent)) implement their
//! bus trait by translating bus primitives onto these three calls and holding
//! only the bus *binding* (I2C address, SPI chip-select pin). Adding a new bus
//! is a new binding + a thin adapter — never a new behavioral engine.
//!
//! Two behaviors are supported, selected by the spec and orthogonal to the bus:
//!
//! - **Register** — a flat register file with pointer/auto-increment semantics,
//!   wide-register read phasing, update rules, and named observables;
//! - **Response** — a fixed-width big-endian word composed from named
//!   bit-fields and clocked out MSB-first on every frame (read-only sensors).

use labwired_ir::component::{IrComponent, IrObservableValue, IrUpdateAction, IrUpdateTrigger};

/// Mutable per-field state for the Response behavior, parallel to
/// `spec.response.fields`.
struct FieldState {
    name: String,
    shift: u8,
    bits: u8,
    /// Raw value in field units; host stimulus mutates this via
    /// [`set_field`](IrCore::set_field).
    value: i64,
}

/// Which behavior the spec describes. Independent of the bus binding.
enum Behavior {
    /// Register-file device (pointer, wide registers, observables, updates).
    Register,
    /// Read-only response-word device.
    Response { byte_index: u8, bytes: u8 },
}

/// Bus-agnostic interpreter for one declarative component instance.
pub struct IrCore {
    spec: IrComponent,
    // ── Register behavior ──
    regs: Vec<u8>,
    wide: Vec<u16>, // parallel to spec.wide_registers; raw 16-bit register image
    pointer: u8,
    read_phase: u8, // 0 = MSB next (wide reads); reset on frame boundary
    writes_since_frame: u32,
    // ── Response behavior ──
    fields: Vec<FieldState>,
    behavior: Behavior,
}

impl IrCore {
    /// Build an engine from a spec. Validation (the same diagnostics surfaced
    /// to agents) gates construction so the interpreter never executes a spec
    /// it cannot run deterministically.
    pub fn new(spec: IrComponent) -> Result<Self, String> {
        let diags = spec.validate();
        if !diags.is_empty() {
            return Err(diags
                .iter()
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("; "));
        }
        let mut regs = vec![0u8; spec.register_file.size];
        for (&off, &v) in &spec.register_file.reset {
            regs[off as usize] = v;
        }
        let wide = spec.wide_registers.iter().map(|w| w.reset).collect();
        let (fields, behavior) = match &spec.response {
            Some(resp) => {
                let fields = resp
                    .fields
                    .iter()
                    .map(|f| FieldState {
                        name: f.name.clone(),
                        shift: f.shift,
                        bits: f.bits,
                        value: f.reset,
                    })
                    .collect();
                (
                    fields,
                    Behavior::Response {
                        byte_index: 0,
                        bytes: resp.bytes,
                    },
                )
            }
            None => (Vec::new(), Behavior::Register),
        };
        Ok(Self {
            regs,
            wide,
            pointer: 0,
            read_phase: 0,
            writes_since_frame: 0,
            fields,
            behavior,
            spec,
        })
    }

    /// A transaction boundary: I2C START, SPI CS edge, etc.
    pub fn reset_frame(&mut self) {
        self.writes_since_frame = 0;
        self.read_phase = 0;
        if let Behavior::Response { byte_index, .. } = &mut self.behavior {
            *byte_index = 0;
        }
    }

    /// One master→device byte.
    pub fn write(&mut self, data: u8) {
        // Response devices are read-only: master bytes carry no state.
        if matches!(self.behavior, Behavior::Response { .. }) {
            return;
        }
        let pointered = self
            .spec
            .pointer
            .as_ref()
            .map(|p| p.first_write_after_start_sets_pointer)
            .unwrap_or(false);
        if pointered && self.writes_since_frame == 0 {
            let mask = self.spec.pointer.as_ref().unwrap().pointer_mask;
            self.pointer = data & mask;
        } else if self.wide_index(self.pointer).is_some() {
            // Absorb: the current pointer selects a wide (multi-byte) register.
            // Wide registers have no writable byte representation; data writes
            // after the pointer-select are silently discarded, matching the
            // reference behavior for read-only wide-register devices (e.g.
            // TMP102 config register: the host may send config bytes that the
            // simulator ignores, preserving the reset value intact).
        } else {
            let idx = self.pointer as usize % self.regs.len().max(1);
            if !self.regs.is_empty() {
                self.regs[idx] = data;
            }
            if self.auto_increment_enabled() {
                self.pointer = self.pointer.wrapping_add(1);
            }
        }
        self.writes_since_frame = self.writes_since_frame.saturating_add(1);
    }

    /// One device→master byte.
    pub fn read(&mut self) -> u8 {
        // Copy the response cursor out first so the immutable borrow of
        // `self.behavior` ends before `compose_word`/the mutable advance below.
        let resp = match &self.behavior {
            Behavior::Response { byte_index, bytes } => Some((*byte_index, *bytes)),
            Behavior::Register => None,
        };
        if let Some((byte_index, bytes)) = resp {
            let word = self.compose_word();
            let last = bytes.saturating_sub(1);
            let idx = byte_index.min(last);
            // MSB-first: byte 0 is the most-significant byte of the word.
            let shift = 8 * (last - idx) as u32;
            let out = ((word >> shift) & 0xFF) as u8;
            if let Behavior::Response { byte_index, .. } = &mut self.behavior {
                if *byte_index < last {
                    *byte_index += 1;
                }
            }
            return out;
        }
        if let Some(wi) = self.wide_index(self.pointer) {
            let value = self.wide[wi];
            let byte = if self.read_phase == 0 {
                (value >> 8) as u8
            } else {
                (value & 0xFF) as u8
            };
            self.read_phase ^= 1;
            if self.read_phase == 0 {
                let ptr = self.pointer;
                self.apply_updates_on_wide_read_complete(ptr);
            }
            return byte;
        }
        if self.regs.is_empty() {
            return 0;
        }
        let idx = self.pointer as usize % self.regs.len();
        let v = self.regs[idx];
        if self.auto_increment_enabled() {
            self.pointer = self.pointer.wrapping_add(1);
        }
        v
    }

    // ── Response-behavior host stimulus ──────────────────────────────────────

    /// Compose the current Response word from field state. Returns 0 for a
    /// Register-behavior device (which has no fields).
    fn compose_word(&self) -> u64 {
        let mut word = 0u64;
        for f in &self.fields {
            // Low `bits` bits of the (possibly negative) value — two's-complement
            // truncation matches a hand-written `(v as uN) & mask` exactly.
            let mask = if f.bits >= 64 {
                u64::MAX
            } else {
                (1u64 << f.bits) - 1
            };
            word |= ((f.value as u64) & mask) << f.shift;
        }
        word
    }

    /// Set a Response field's raw value by name. Returns `false` for an unknown
    /// field (or a Register-behavior device).
    pub fn set_field(&mut self, name: &str, value: i64) -> bool {
        if let Some(f) = self.fields.iter_mut().find(|f| f.name == name) {
            f.value = value;
            true
        } else {
            false
        }
    }

    /// Read back a Response field's raw value, or `None` if unknown.
    pub fn field(&self, name: &str) -> Option<i64> {
        self.fields.iter().find(|f| f.name == name).map(|f| f.value)
    }

    // ── Register-behavior internals (ported verbatim) ────────────────────────

    fn auto_increment_enabled(&self) -> bool {
        use labwired_ir::component::IrAutoIncrement;
        match self.spec.pointer.as_ref().map(|p| &p.auto_increment) {
            Some(IrAutoIncrement::Always) => true,
            Some(IrAutoIncrement::WhenFieldSet { reg, mask }) => {
                self.regs.get(*reg as usize).is_some_and(|r| r & mask != 0)
            }
            Some(IrAutoIncrement::Never) | None => false,
        }
    }

    fn wide_index(&self, pointer: u8) -> Option<usize> {
        self.spec
            .wide_registers
            .iter()
            .position(|w| w.pointer == pointer)
    }

    /// Read a named observable for a specific channel. See
    /// [`IrI2cComponent::observable`](super::ir_component::IrI2cComponent::observable).
    pub fn observable(&self, name: &str, channel: u8) -> Option<f32> {
        let obs = self.spec.observables.iter().find(|o| o.name == name)?;
        if channel >= obs.channels {
            return None;
        }
        let base = obs.base as usize + obs.stride as usize * channel as usize;
        let IrObservableValue::U12Compose {
            lo_rel,
            hi_rel,
            hi_mask,
        } = obs.value;
        let lo = self.regs[base + lo_rel as usize];
        let hi = self.regs[base + hi_rel as usize] & hi_mask;
        let raw = ((hi as u16) << 8) | lo as u16;
        if let Some(map) = &obs.map {
            if map.none_when_raw_zero && raw == 0 {
                return None;
            }
            let eng = raw as f32 * map.linear.scale + map.linear.offset;
            let eng = if let Some((lo_clamp, hi_clamp)) = map.linear.clamp {
                eng.clamp(lo_clamp, hi_clamp)
            } else {
                eng
            };
            Some(eng)
        } else {
            Some(raw as f32)
        }
    }

    fn apply_updates_on_wide_read_complete(&mut self, pointer: u8) {
        // Collect first to satisfy the borrow checker; specs are small.
        let actions: Vec<IrUpdateAction> = self
            .spec
            .updates
            .iter()
            .filter(|u| {
                matches!(u.trigger, IrUpdateTrigger::WideReadComplete { pointer: p } if p == pointer)
            })
            .map(|u| u.action.clone())
            .collect();
        if actions.is_empty() {
            return;
        }
        let wi = self.wide_index(pointer).expect("validated");
        for a in actions {
            let IrUpdateAction::AddWrap { add, max, reset } = a;
            // Storage is the raw 16-bit register image; AddWrap is defined as
            // signed per the IR doc ("signed compare, as i16"), so the signed
            // view exists only here — registers with bit 15 set remain
            // well-defined (e.g. reset=0xE700 stays negative after wrapping add).
            let mut v = (self.wide[wi] as i16).wrapping_add(add);
            if v > max {
                v = reset;
            }
            self.wide[wi] = v as u16;
        }
    }
}
