// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal WebAssembly **binary** encoder for the RV32IMC JIT emit core.
//!
//! The JIT frontend must produce a [`BlockPlan.code`](super::super::frontend::BlockPlan)
//! that is a *real* wasm module byte stream — the same bytes the native
//! (`wasmtime`) and (future) browser (`js_sys::WebAssembly`) runtimes both
//! consume. This module is the smallest self-contained encoder that emits
//! exactly the module shape the emit core needs:
//!
//! ```text
//! (module
//!   (import "regs" "mem" (memory 1))          ;; the guest register file
//!   (func (export "run") (result i32)
//!     (local i32 ... 32×)                       ;; x0..x31 held in locals
//!     <prologue: load touched regs from mem>
//!     <body: one emit per guest ALU instruction>
//!     <epilogue: store written regs back to mem>
//!     (i32.const <wire-code>)))                 ;; side-exit wire code
//! ```
//!
//! It is deliberately tiny — no data/global/table sections, one function,
//! one imported memory — and always compiled (no `wasmtime` dependency), so
//! the browser build can share it verbatim when that runtime lands. The
//! wasm opcodes the emit core uses are exposed as [`op`] constants; the
//! [`ModuleBuilder`] frames the fixed section skeleton around a caller-
//! supplied code body.

/// LEB128 / opcode primitives shared by the emit core.
pub mod enc {
    /// Append an unsigned LEB128 encoding of `value`.
    pub fn uleb(buf: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    /// Append a signed LEB128 encoding of `value` (covers both `i32.const`
    /// and `i64.const` operands — the encoding is width-agnostic).
    pub fn sleb(buf: &mut Vec<u8>, mut value: i64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7; // arithmetic shift keeps the sign bit
            let sign_bit_set = (byte & 0x40) != 0;
            let more = !((value == 0 && !sign_bit_set) || (value == -1 && sign_bit_set));
            buf.push(if more { byte | 0x80 } else { byte & 0x7f });
            if !more {
                break;
            }
        }
    }
}

/// WebAssembly opcode / type-tag byte constants used by the emit core.
///
/// Only the subset the RV32IMC ALU codegen emits is listed. Grouped by role
/// to keep the emit core readable.
#[allow(dead_code)]
pub mod op {
    // ── value / type tags ──────────────────────────────────────────────
    /// `i32` value type tag (also the `if`/`block` result type for i32).
    pub const T_I32: u8 = 0x7f;
    /// Empty block type (`if`/`block` producing no value): `0x40`.
    pub const T_EMPTY: u8 = 0x40;

    // ── locals / constants ─────────────────────────────────────────────
    pub const LOCAL_GET: u8 = 0x20;
    pub const LOCAL_SET: u8 = 0x21;
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const DROP: u8 = 0x1a;

    // ── structured control flow ────────────────────────────────────────
    pub const IF: u8 = 0x04;
    pub const ELSE: u8 = 0x05;
    pub const END: u8 = 0x0b;
    /// Early function return with the current stack as the result.
    pub const RETURN: u8 = 0x0f;

    // ── memory (memarg = align:uleb offset:uleb; align is a log2 hint) ──
    pub const I32_LOAD: u8 = 0x28;
    pub const I32_LOAD8_S: u8 = 0x2c;
    pub const I32_LOAD8_U: u8 = 0x2d;
    pub const I32_LOAD16_S: u8 = 0x2e;
    pub const I32_LOAD16_U: u8 = 0x2f;
    pub const I32_STORE: u8 = 0x36;
    pub const I32_STORE8: u8 = 0x3a;
    pub const I32_STORE16: u8 = 0x3b;

    // ── i32 arithmetic / logic ─────────────────────────────────────────
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_LT_U: u8 = 0x49;
    pub const I32_LE_U: u8 = 0x4d;
    pub const I32_GE_U: u8 = 0x4f;
    pub const I32_ADD: u8 = 0x6a;
    pub const I32_SUB: u8 = 0x6b;
    pub const I32_MUL: u8 = 0x6c;
    pub const I32_DIV_S: u8 = 0x6d;
    pub const I32_DIV_U: u8 = 0x6e;
    pub const I32_REM_S: u8 = 0x6f;
    pub const I32_REM_U: u8 = 0x70;
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    pub const I32_SHL: u8 = 0x74;
    pub const I32_SHR_S: u8 = 0x75;
    pub const I32_SHR_U: u8 = 0x76;

    // ── i64 (used only by the MULH family) ─────────────────────────────
    pub const I64_MUL: u8 = 0x7e;
    pub const I64_SHR_U: u8 = 0x88;
    pub const I64_EXTEND_I32_S: u8 = 0xac;
    pub const I64_EXTEND_I32_U: u8 = 0xad;
    pub const I32_WRAP_I64: u8 = 0xa7;
}

/// The name of the imported memory that backs the guest register file:
/// `(import "regs" "mem" (memory 1))`. Word `i` (register `xi`) lives at
/// byte offset `i * 4`.
pub const REGS_IMPORT_MODULE: &str = "regs";
/// Field name of the imported register-file memory (see
/// [`REGS_IMPORT_MODULE`]).
pub const REGS_IMPORT_FIELD: &str = "mem";
/// Name the compiled block's entry function is exported under.
pub const RUN_EXPORT: &str = "run";

/// Assemble the fixed module skeleton around a code body.
///
/// The skeleton is constant across every compiled block (one imported
/// memory, one exported `run` function returning `i32`); only the code body,
/// the local count, and the imported memory's minimum page count vary.
/// `local_i32_count` is the number of `i32` locals the body references (the
/// emit core uses 32 for `x0..x31`, plus one scratch when it emits memory
/// ops). `mem_min_pages` is the minimum size (64 KiB pages) the imported
/// memory must declare — 1 for a register-only (pure-ALU) block, or enough
/// to cover the guest-RAM window a load/store block binds against (see
/// [`super::emit`]).
pub fn build_module(local_i32_count: u32, mem_min_pages: u32, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(64 + body.len());
    // Magic + version.
    m.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);

    // ── Type section (id 1): one type `() -> i32`. ─────────────────────
    {
        let mut c = Vec::new();
        enc::uleb(&mut c, 1); // one type
        c.push(0x60); // functype
        enc::uleb(&mut c, 0); // 0 params
        enc::uleb(&mut c, 1); // 1 result
        c.push(op::T_I32);
        section(&mut m, 1, &c);
    }

    // ── Import section (id 2): (import "regs" "mem" (memory 1)). ────────
    {
        let mut c = Vec::new();
        enc::uleb(&mut c, 1); // one import
        name(&mut c, REGS_IMPORT_MODULE);
        name(&mut c, REGS_IMPORT_FIELD);
        c.push(0x02); // import kind: memory
        c.push(0x00); // limits: min only (no max)
        enc::uleb(&mut c, mem_min_pages as u64); // min pages
        section(&mut m, 2, &c);
    }

    // ── Function section (id 3): one function of type 0. ───────────────
    {
        let mut c = Vec::new();
        enc::uleb(&mut c, 1);
        enc::uleb(&mut c, 0); // type index 0
        section(&mut m, 3, &c);
    }

    // ── Export section (id 7): (export "run" (func 0)). ────────────────
    {
        let mut c = Vec::new();
        enc::uleb(&mut c, 1);
        name(&mut c, RUN_EXPORT);
        c.push(0x00); // export kind: func
        enc::uleb(&mut c, 0); // func index 0
        section(&mut m, 7, &c);
    }

    // ── Code section (id 10): one function body. ───────────────────────
    {
        // The body proper: local declarations then the caller expression,
        // terminated by `end`.
        let mut func = Vec::with_capacity(body.len() + 8);
        if local_i32_count == 0 {
            enc::uleb(&mut func, 0); // no local groups
        } else {
            enc::uleb(&mut func, 1); // one local group
            enc::uleb(&mut func, local_i32_count as u64);
            func.push(op::T_I32);
        }
        func.extend_from_slice(body);
        func.push(op::END);

        let mut c = Vec::new();
        enc::uleb(&mut c, 1); // one code entry
        enc::uleb(&mut c, func.len() as u64);
        c.extend_from_slice(&func);
        section(&mut m, 10, &c);
    }

    m
}

/// Frame `content` as a section with the given `id`.
fn section(out: &mut Vec<u8>, id: u8, content: &[u8]) {
    out.push(id);
    enc::uleb(out, content.len() as u64);
    out.extend_from_slice(content);
}

/// Append a wasm `name` (length-prefixed UTF-8).
fn name(out: &mut Vec<u8>, s: &str) {
    enc::uleb(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uleb_roundtrip_small_and_multibyte() {
        let mut b = Vec::new();
        enc::uleb(&mut b, 0);
        assert_eq!(b, vec![0x00]);
        b.clear();
        enc::uleb(&mut b, 127);
        assert_eq!(b, vec![0x7f]);
        b.clear();
        enc::uleb(&mut b, 128);
        assert_eq!(b, vec![0x80, 0x01]);
        b.clear();
        enc::uleb(&mut b, 624_485);
        assert_eq!(b, vec![0xE5, 0x8E, 0x26]);
    }

    #[test]
    fn sleb_roundtrip_signed() {
        // Canonical examples from the LEB128 spec.
        let mut b = Vec::new();
        enc::sleb(&mut b, 0);
        assert_eq!(b, vec![0x00]);
        b.clear();
        enc::sleb(&mut b, -1);
        assert_eq!(b, vec![0x7f]);
        b.clear();
        enc::sleb(&mut b, 63);
        assert_eq!(b, vec![0x3f]);
        b.clear();
        enc::sleb(&mut b, 64);
        assert_eq!(b, vec![0xC0, 0x00]);
        b.clear();
        enc::sleb(&mut b, -64);
        assert_eq!(b, vec![0x40]);
        b.clear();
        enc::sleb(&mut b, i32::MIN as i64);
        assert_eq!(b, vec![0x80, 0x80, 0x80, 0x80, 0x78]);
    }

    #[test]
    fn empty_body_module_has_wasm_magic() {
        // `(i32.const 0)` body — the minimal valid `run`.
        let body = vec![op::I32_CONST, 0x00];
        let m = build_module(0, 1, &body);
        assert_eq!(&m[0..4], &[0x00, 0x61, 0x73, 0x6d], "wasm magic");
        assert_eq!(&m[4..8], &[0x01, 0x00, 0x00, 0x00], "wasm version 1");
        // Section ids appear in canonical ascending order.
        // (type=1, import=2, func=3, export=7, code=10)
        assert!(m.contains(&10u8), "code section present");
    }
}
