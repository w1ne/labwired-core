// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Universal peripheral inspection interface (Slice 1 — core machinery).
//!
//! A uniform, schema-driven way to read any peripheral's decoded
//! register/artifact state. The design intent (see `debug-inspect-proposal.md`):
//!
//! * **Snapshot semantics only** — inspect reads the post-run / paused machine
//!   state; it is not a live stepped debugger.
//! * **Side-effect-free decode** — [`default_inspect`] reads register words via
//!   [`Peripheral::peek`], never `read()`, so inspecting a read-to-clear
//!   register never perturbs it.
//! * **Honest gaps** — [`crate::Machine::peek`] returns an explicit
//!   [`PeekByte::Unmapped`] marker for unmodeled address space instead of silent
//!   zeros, so unmapped regions never look like real data.
//! * **Summary mode by default** — big artifact payloads (framebuffers) are
//!   omitted unless [`InspectOpts::include_bytes`] is set; a cheap
//!   `meta.generation` hash lets callers skip re-pulling unchanged buffers.
//!
//! The highest-leverage piece is [`default_inspect`]: any peripheral that
//! returns a schema from [`Peripheral::describe_registers`] (every declarative
//! `GenericPeripheral` — the whole ESP32-C3/S3 register wall) gets named,
//! field-decoded registers for zero bespoke code.

use crate::Peripheral;
use serde::{Deserialize, Serialize};

/// One field within a register schema: a named bit slice `[msb, lsb]`.
///
/// Mirrors [`labwired_config::FieldDescriptor`] but in the inspect vocabulary,
/// decoupled from the on-disk config format.
#[derive(Debug, Clone, Serialize)]
pub struct FieldSchema {
    pub name: String,
    /// `[msb, lsb]`, inclusive.
    pub bits: [u8; 2],
}

/// The register-layout schema a peripheral advertises for decoding.
///
/// Mirrors [`labwired_config::RegisterDescriptor`]. Declarative peripherals
/// return this straight from their descriptor; native peripherals may return a
/// static map or `None` (then inspect yields registers with no schema).
#[derive(Debug, Clone, Serialize)]
pub struct RegisterSchema {
    pub name: String,
    pub offset: u64,
    /// Bit width: 8, 16, or 32.
    pub size: u8,
    /// `"rw"` | `"ro"` | `"wo"`.
    pub access: &'static str,
    pub fields: Vec<FieldSchema>,
}

/// A decoded field value: the schema slice plus its extracted value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldView {
    pub name: String,
    /// `[msb, lsb]`, inclusive.
    pub bits: [u8; 2],
    pub value: u32,
}

/// One decoded register: the live raw word plus schema-decoded fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterView {
    pub name: String,
    pub offset: u64,
    /// Bit width: 8, 16, or 32.
    pub size: u8,
    /// Live raw word, read side-effect-free via [`Peripheral::peek`].
    pub value: u32,
    /// Decoded via `bit_range`; empty when the register carries no field schema.
    pub fields: Vec<FieldView>,
    /// `"rw"` | `"ro"` | `"wo"`.
    pub access: String,
}

/// A typed non-register artifact (framebuffer, uart ring, bus trace, pins …).
///
/// Large payloads live in `bytes`, which is omitted in summary mode; callers
/// use `meta.generation` (a cheap content hash) to detect changes without
/// re-pulling the bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// `"framebuffer"` | `"uart"` | `"bus_trace"` | `"pins"` | …
    pub kind: String,
    /// Device / stream id.
    pub id: String,
    /// `{ width, height, format, generation, … }`.
    pub meta: serde_json::Value,
    /// Large payload; present only when `include_bytes` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

/// A single peripheral's decoded state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeripheralInspect {
    pub name: String,
    /// Coarse kind: `"declarative"` | `"native"` | `"i2c"` | …
    pub kind: String,
    pub base: u64,
    pub registers: Vec<RegisterView>,
    pub artifacts: Vec<Artifact>,
}

/// The whole machine's decoded state (or a single filtered peripheral).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineInspect {
    pub peripherals: Vec<PeripheralInspect>,
}

/// Options controlling an inspect walk.
#[derive(Debug, Clone, Default)]
pub struct InspectOpts {
    /// When `true`, artifacts carry their full byte payload; otherwise summary
    /// mode (metadata + generation hash only).
    pub include_bytes: bool,
    /// Restrict the walk to a single peripheral by name. `None` = all.
    pub peripheral: Option<String>,
}

/// One byte of a [`crate::Machine::peek`] read.
///
/// Modeled space yields [`PeekByte::Mapped`]; a gap (no memory region and no
/// peripheral window covers the address) yields [`PeekByte::Unmapped`] — never a
/// silent zero, so unmodeled space cannot be mistaken for real data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeekByte {
    Mapped(u8),
    Unmapped,
}

/// The result of a [`crate::Machine::peek`]: `len` bytes starting at `addr`,
/// each carrying an explicit mapped/unmapped marker.
#[derive(Debug, Clone, Serialize)]
pub struct PeekResult {
    pub addr: u64,
    pub bytes: Vec<PeekByte>,
}

impl PeekResult {
    /// Collapse to raw bytes, substituting `0` for unmapped positions. Used by
    /// the wasm raw escape hatch, which returns a plain byte buffer; honest
    /// callers use [`PeekResult::bytes`] directly.
    pub fn to_lossy_bytes(&self) -> Vec<u8> {
        self.bytes
            .iter()
            .map(|b| match b {
                PeekByte::Mapped(v) => *v,
                PeekByte::Unmapped => 0,
            })
            .collect()
    }
}

/// Extract the value of a `[msb, lsb]` (inclusive) bit slice from `word`.
fn extract_field(word: u32, bits: [u8; 2]) -> u32 {
    let msb = bits[0].min(31);
    let lsb = bits[1].min(msb);
    let width = msb - lsb + 1;
    let mask = if width >= 32 {
        u32::MAX
    } else {
        (1u32 << width) - 1
    };
    (word >> lsb) & mask
}

/// Assemble a little-endian register word of `size` bits from side-effect-free
/// [`Peripheral::peek`] byte reads. Bytes the peripheral can't probe read as 0.
fn peek_word<P: Peripheral + ?Sized>(p: &P, offset: u64, size: u8) -> u32 {
    let n = (size / 8).max(1) as u64;
    let mut word: u32 = 0;
    for i in 0..n {
        let byte = p.peek(offset + i).unwrap_or(0) as u32;
        word |= byte << (8 * i);
    }
    word
}

/// Generic peripheral inspection: walk the register schema, decode each word
/// (side-effect-free via `peek`) and its named fields.
///
/// This is the default body of [`Peripheral::inspect`]. Peripherals that expose
/// non-register artifacts (framebuffers, traces) override `inspect`, typically
/// by calling `default_inspect` and pushing artifacts onto the result.
///
/// Generic over `?Sized` so the trait default can pass `self` (a `&dyn
/// Peripheral`) through without a `Sized` bound — keeping `inspect`
/// object-safe.
pub fn default_inspect<P: Peripheral + ?Sized>(
    p: &P,
    base: u64,
    name: &str,
    _opts: &InspectOpts,
) -> PeripheralInspect {
    let schema = p.describe_registers();
    let kind = if schema.is_some() {
        "declarative"
    } else {
        "native"
    };

    let mut registers = Vec::new();
    if let Some(schema) = schema {
        for reg in schema {
            let value = peek_word(p, reg.offset, reg.size);
            let fields = reg
                .fields
                .iter()
                .map(|f| FieldView {
                    name: f.name.clone(),
                    bits: f.bits,
                    value: extract_field(value, f.bits),
                })
                .collect();
            registers.push(RegisterView {
                name: reg.name,
                offset: reg.offset,
                size: reg.size,
                value,
                fields,
                access: reg.access.to_string(),
            });
        }
    }

    PeripheralInspect {
        name: name.to_string(),
        kind: kind.to_string(),
        base,
        registers,
        artifacts: Vec::new(),
    }
}

/// FNV-1a hash of a byte buffer, used as a cheap `meta.generation` so callers
/// can detect an unchanged artifact without re-pulling its bytes.
pub fn artifact_generation(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_field_slices_bits() {
        // 0b1101_10 -> value 0b110110 = 54
        let word = 0b0000_0000_0000_0000_0000_0000_0011_0110;
        assert_eq!(extract_field(word, [1, 0]), 0b10);
        assert_eq!(extract_field(word, [2, 2]), 0b1);
        assert_eq!(extract_field(word, [5, 0]), 0b110110);
        assert_eq!(extract_field(word, [31, 0]), word);
    }

    #[test]
    fn generation_changes_with_bytes() {
        assert_ne!(
            artifact_generation(&[0, 0, 0]),
            artifact_generation(&[0, 1, 0])
        );
        assert_eq!(
            artifact_generation(&[1, 2, 3]),
            artifact_generation(&[1, 2, 3])
        );
    }
}
