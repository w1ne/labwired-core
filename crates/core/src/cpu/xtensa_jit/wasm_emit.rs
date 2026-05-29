// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Hand-rolled wasm binary encoder for the universal browser JIT.
//!
//! Replaces the build-time `wat`-baked single-block path with runtime
//! byte emission. Implements only the subset of the wasm binary format
//! the JIT actually needs: type / import / function / export / code
//! sections + LEB128 immediates. No tables, globals, element, data,
//! memory, or start sections — JIT'd blocks don't use any of those.
//!
//! Reference: WebAssembly Core Specification §5 (Binary Format).

/// Append a value as unsigned LEB128.
pub fn encode_u32(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Append a value as signed LEB128.
pub fn encode_s32(mut value: i32, out: &mut Vec<u8>) {
    loop {
        let byte = (value as u8) & 0x7F;
        // Arithmetic shift right preserves sign.
        value >>= 7;
        // Termination: value has stabilized at 0 or -1 AND the sign bit of the
        // emitted byte matches.
        let sign_bit = byte & 0x40;
        if (value == 0 && sign_bit == 0) || (value == -1 && sign_bit != 0) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

// ── Wasm value types (subset we use) ──────────────────────────────────────────

/// Wasm value type encoding.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValType {
    I32 = 0x7F,
    I64 = 0x7E,
}

impl ValType {
    fn byte(self) -> u8 {
        self as u8
    }
}

/// A wasm function type: parameter types and result types.
#[derive(Clone, Debug)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

// ── Module builder ────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct WasmModule {
    types: Vec<FuncType>,
    /// (module, name, type_idx) for each function import.
    imports: Vec<(String, String, u32)>,
    /// type_idx per defined function (excluding imports).
    func_type_idx: Vec<u32>,
    /// (name, func_idx_global) for each exported function.
    exports: Vec<(String, u32)>,
    /// Raw code body per defined function: just the body bytes (locals
    /// + instructions), without the size prefix or the end marker — the
    /// `finish_code_section` adds those.
    codes: Vec<Vec<u8>>,
}

impl WasmModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a function type; returns its index.
    pub fn add_type(&mut self, ty: FuncType) -> u32 {
        let idx = self.types.len() as u32;
        self.types.push(ty);
        idx
    }

    /// Declare a function import; returns its global function index.
    /// Function imports are indexed before defined functions.
    pub fn add_func_import(&mut self, module: &str, name: &str, type_idx: u32) -> u32 {
        let idx = self.imports.len() as u32;
        self.imports
            .push((module.to_string(), name.to_string(), type_idx));
        idx
    }

    /// Declare a defined function with body bytes; returns its global
    /// function index. Body bytes must be the function body (without
    /// locals-count or end byte — those are appended here).
    pub fn add_func(&mut self, type_idx: u32, body: Vec<u8>) -> u32 {
        let idx = (self.imports.len() + self.func_type_idx.len()) as u32;
        self.func_type_idx.push(type_idx);
        self.codes.push(body);
        idx
    }

    /// Export a function by global index.
    pub fn add_func_export(&mut self, name: &str, func_idx: u32) {
        self.exports.push((name.to_string(), func_idx));
    }

    /// Finalize: emit the binary module bytes.
    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        // Magic + version.
        out.extend_from_slice(b"\0asm");
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        // Sections in spec order. Each section is `id, size-leb, contents`.
        if !self.types.is_empty() {
            push_section(&mut out, 1, |s| {
                encode_u32(self.types.len() as u32, s);
                for ty in &self.types {
                    s.push(0x60); // func type tag
                    encode_u32(ty.params.len() as u32, s);
                    for p in &ty.params {
                        s.push(p.byte());
                    }
                    encode_u32(ty.results.len() as u32, s);
                    for r in &ty.results {
                        s.push(r.byte());
                    }
                }
            });
        }
        if !self.imports.is_empty() {
            push_section(&mut out, 2, |s| {
                encode_u32(self.imports.len() as u32, s);
                for (mod_name, name, ty_idx) in &self.imports {
                    encode_u32(mod_name.len() as u32, s);
                    s.extend_from_slice(mod_name.as_bytes());
                    encode_u32(name.len() as u32, s);
                    s.extend_from_slice(name.as_bytes());
                    s.push(0x00); // import kind = func
                    encode_u32(*ty_idx, s);
                }
            });
        }
        if !self.func_type_idx.is_empty() {
            push_section(&mut out, 3, |s| {
                encode_u32(self.func_type_idx.len() as u32, s);
                for ty_idx in &self.func_type_idx {
                    encode_u32(*ty_idx, s);
                }
            });
        }
        if !self.exports.is_empty() {
            push_section(&mut out, 7, |s| {
                encode_u32(self.exports.len() as u32, s);
                for (name, fn_idx) in &self.exports {
                    encode_u32(name.len() as u32, s);
                    s.extend_from_slice(name.as_bytes());
                    s.push(0x00); // export kind = func
                    encode_u32(*fn_idx, s);
                }
            });
        }
        if !self.codes.is_empty() {
            push_section(&mut out, 10, |s| {
                encode_u32(self.codes.len() as u32, s);
                for body in &self.codes {
                    // Body = locals-vec + instructions + end. We require
                    // the caller to have included locals + end already if
                    // it wanted them — the body is taken verbatim, then
                    // size-prefixed.
                    encode_u32(body.len() as u32, s);
                    s.extend_from_slice(body);
                }
            });
        }
        out
    }
}

/// Emit one section: id byte, then size-prefixed contents.
fn push_section(out: &mut Vec<u8>, id: u8, build: impl FnOnce(&mut Vec<u8>)) {
    let mut section = Vec::with_capacity(32);
    build(&mut section);
    out.push(id);
    encode_u32(section.len() as u32, out);
    out.extend_from_slice(&section);
}

// ── Instruction byte emitters (Wasm Core Spec §5.4) ───────────────────────────
//
// Each helper appends the opcode byte (and any LEB-encoded immediates) to
// `out`. Numeric and parametric ops are single-byte; variable / control /
// call ops carry an unsigned (or, for blocktype, signed) LEB128 immediate.

/// `local.get x` — push the value of local `x`.
pub fn emit_local_get(idx: u32, out: &mut Vec<u8>) {
    out.push(0x20);
    encode_u32(idx, out);
}

/// `local.set x` — pop and store into local `x`.
pub fn emit_local_set(idx: u32, out: &mut Vec<u8>) {
    out.push(0x21);
    encode_u32(idx, out);
}

/// `local.tee x` — store top-of-stack into local `x` without popping.
pub fn emit_local_tee(idx: u32, out: &mut Vec<u8>) {
    out.push(0x22);
    encode_u32(idx, out);
}

/// `i32.const N` — push a 32-bit signed immediate (LEB128-encoded).
pub fn emit_i32_const(value: i32, out: &mut Vec<u8>) {
    out.push(0x41);
    encode_s32(value, out);
}

/// `i32.add` — pop two i32s, push their sum.
pub fn emit_i32_add(out: &mut Vec<u8>) {
    out.push(0x6A);
}

/// `i32.sub` — pop b then a, push a - b.
pub fn emit_i32_sub(out: &mut Vec<u8>) {
    out.push(0x6B);
}

/// `i32.and` — bitwise AND.
pub fn emit_i32_and(out: &mut Vec<u8>) {
    out.push(0x71);
}

/// `i32.or` — bitwise OR.
pub fn emit_i32_or(out: &mut Vec<u8>) {
    out.push(0x72);
}

/// `i32.xor` — bitwise XOR.
pub fn emit_i32_xor(out: &mut Vec<u8>) {
    out.push(0x73);
}

/// `i32.shl` — logical shift left.
pub fn emit_i32_shl(out: &mut Vec<u8>) {
    out.push(0x74);
}

/// `i32.shr_s` — arithmetic (signed) shift right.
pub fn emit_i32_shr_s(out: &mut Vec<u8>) {
    out.push(0x75);
}

/// `i32.shr_u` — logical (unsigned) shift right.
pub fn emit_i32_shr_u(out: &mut Vec<u8>) {
    out.push(0x76);
}

/// `i32.eqz` — push 1 if top-of-stack is zero, else 0.
pub fn emit_i32_eqz(out: &mut Vec<u8>) {
    out.push(0x45);
}

/// `i32.eq` — push 1 if a == b, else 0.
pub fn emit_i32_eq(out: &mut Vec<u8>) {
    out.push(0x46);
}

/// `i32.lt_s` — signed less-than comparison.
pub fn emit_i32_lt_s(out: &mut Vec<u8>) {
    out.push(0x48);
}

/// `call x` — invoke function index `x`.
pub fn emit_call(func_idx: u32, out: &mut Vec<u8>) {
    out.push(0x10);
    encode_u32(func_idx, out);
}

/// `return` — return from the current function.
pub fn emit_return(out: &mut Vec<u8>) {
    out.push(0x0F);
}

/// `drop` — pop and discard top-of-stack.
pub fn emit_drop(out: &mut Vec<u8>) {
    out.push(0x1A);
}

/// `end` — end of a block / function body.
pub fn emit_end(out: &mut Vec<u8>) {
    out.push(0x0B);
}

/// `else` — start the else-branch of an `if`.
pub fn emit_else(out: &mut Vec<u8>) {
    out.push(0x05);
}

/// `if blocktype` where blocktype = type index (signed-LEB-encoded per §5.3.2).
///
/// Used for multi-value `if` blocks (e.g. the JIT's return-on-error path
/// produces 5 × i32). Per the Wasm Core Spec, blocktype carries an `s33`
/// when it refers to a type index — small positive indices encode
/// identically to unsigned LEB, but indices ≥ 64 differ (signed LEB inserts
/// a zero continuation byte to keep the sign bit clear). We delegate to
/// `encode_s32`; this is valid for indices up to `i32::MAX`, which is more
/// than enough for any JIT'd module.
pub fn emit_if_type(type_idx: u32, out: &mut Vec<u8>) {
    out.push(0x04);
    encode_s32(type_idx as i32, out);
}

/// `if (result <none>)` — single-byte blocktype 0x40 means empty type.
pub fn emit_if_void(out: &mut Vec<u8>) {
    out.push(0x04);
    out.push(0x40);
}

/// Emit one `(count, valtype)` run for the locals declaration vector of a
/// code section body. Wasm encodes a function's locals as a vec of runs;
/// callers that need multiple distinct types should call this once per run
/// (and prefix the whole vector with its run-count via `encode_u32`).
pub fn emit_locals_run(count: u32, ty: ValType, out: &mut Vec<u8>) {
    encode_u32(count, out);
    out.push(ty.byte());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_u(v: u32) -> Vec<u8> {
        let mut out = Vec::new();
        encode_u32(v, &mut out);
        out
    }

    fn enc_s(v: i32) -> Vec<u8> {
        let mut out = Vec::new();
        encode_s32(v, &mut out);
        out
    }

    #[test]
    fn leb128_u32_known_values() {
        // Reference cases from Wasm spec + GitHub gists.
        assert_eq!(enc_u(0), vec![0x00]);
        assert_eq!(enc_u(1), vec![0x01]);
        assert_eq!(enc_u(127), vec![0x7F]);
        assert_eq!(enc_u(128), vec![0x80, 0x01]);
        assert_eq!(enc_u(16383), vec![0xFF, 0x7F]);
        assert_eq!(enc_u(16384), vec![0x80, 0x80, 0x01]);
        assert_eq!(enc_u(624485), vec![0xE5, 0x8E, 0x26]);
        assert_eq!(enc_u(u32::MAX), vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    #[test]
    fn leb128_s32_known_values() {
        assert_eq!(enc_s(0), vec![0x00]);
        assert_eq!(enc_s(1), vec![0x01]);
        assert_eq!(enc_s(-1), vec![0x7F]);
        assert_eq!(enc_s(63), vec![0x3F]);
        assert_eq!(enc_s(-64), vec![0x40]);
        assert_eq!(enc_s(64), vec![0xC0, 0x00]);
        assert_eq!(enc_s(-65), vec![0xBF, 0x7F]);
        assert_eq!(enc_s(-12345), vec![0xC7, 0x9F, 0x7F]);
    }

    #[test]
    fn empty_module_starts_with_magic() {
        let bytes = WasmModule::new().finish();
        assert_eq!(&bytes[0..8], b"\0asm\x01\0\0\0");
        // Empty module = just magic + version, no sections.
        assert_eq!(bytes.len(), 8);
    }

    // ── Instruction emitter tests ────────────────────────────────────────────
    //
    // One test per helper, asserting the exact byte sequence per Wasm Core
    // Spec §5.4. Helper to make these short:
    fn emit<F: FnOnce(&mut Vec<u8>)>(f: F) -> Vec<u8> {
        let mut v = Vec::new();
        f(&mut v);
        v
    }

    #[test]
    fn op_local_get() {
        assert_eq!(emit(|o| emit_local_get(0, o)), vec![0x20, 0x00]);
        // Multi-byte LEB for the immediate.
        assert_eq!(emit(|o| emit_local_get(128, o)), vec![0x20, 0x80, 0x01]);
    }

    #[test]
    fn op_local_set() {
        assert_eq!(emit(|o| emit_local_set(5, o)), vec![0x21, 0x05]);
    }

    #[test]
    fn op_local_tee() {
        assert_eq!(emit(|o| emit_local_tee(5, o)), vec![0x22, 0x05]);
    }

    #[test]
    fn op_i32_const() {
        assert_eq!(emit(|o| emit_i32_const(0, o)), vec![0x41, 0x00]);
        assert_eq!(emit(|o| emit_i32_const(-1, o)), vec![0x41, 0x7F]);
        assert_eq!(emit(|o| emit_i32_const(255, o)), vec![0x41, 0xFF, 0x01]);
        assert_eq!(
            emit(|o| emit_i32_const(-12345, o)),
            vec![0x41, 0xC7, 0x9F, 0x7F]
        );
    }

    #[test]
    fn op_i32_add() {
        assert_eq!(emit(emit_i32_add), vec![0x6A]);
    }

    #[test]
    fn op_i32_sub() {
        assert_eq!(emit(emit_i32_sub), vec![0x6B]);
    }

    #[test]
    fn op_i32_and() {
        assert_eq!(emit(emit_i32_and), vec![0x71]);
    }

    #[test]
    fn op_i32_or() {
        assert_eq!(emit(emit_i32_or), vec![0x72]);
    }

    #[test]
    fn op_i32_xor() {
        assert_eq!(emit(emit_i32_xor), vec![0x73]);
    }

    #[test]
    fn op_i32_shl() {
        assert_eq!(emit(emit_i32_shl), vec![0x74]);
    }

    #[test]
    fn op_i32_shr_s() {
        assert_eq!(emit(emit_i32_shr_s), vec![0x75]);
    }

    #[test]
    fn op_i32_shr_u() {
        assert_eq!(emit(emit_i32_shr_u), vec![0x76]);
    }

    #[test]
    fn op_i32_eqz() {
        assert_eq!(emit(emit_i32_eqz), vec![0x45]);
    }

    #[test]
    fn op_i32_eq() {
        assert_eq!(emit(emit_i32_eq), vec![0x46]);
    }

    #[test]
    fn op_i32_lt_s() {
        assert_eq!(emit(emit_i32_lt_s), vec![0x48]);
    }

    #[test]
    fn op_call() {
        assert_eq!(emit(|o| emit_call(7, o)), vec![0x10, 0x07]);
    }

    #[test]
    fn op_return() {
        assert_eq!(emit(emit_return), vec![0x0F]);
    }

    #[test]
    fn op_drop() {
        assert_eq!(emit(emit_drop), vec![0x1A]);
    }

    #[test]
    fn op_end() {
        assert_eq!(emit(emit_end), vec![0x0B]);
    }

    #[test]
    fn op_else() {
        assert_eq!(emit(emit_else), vec![0x05]);
    }

    #[test]
    fn op_if_void() {
        assert_eq!(emit(emit_if_void), vec![0x04, 0x40]);
    }

    #[test]
    fn op_if_type() {
        // Small positive type idx encodes to a single LEB byte (matches
        // unsigned-LEB for values < 64).
        assert_eq!(emit(|o| emit_if_type(3, o)), vec![0x04, 0x03]);
        // For values ≥ 64 the signed LEB encoding inserts a continuation
        // so the sign bit of the final byte stays clear. 128 → 0x80 0x01.
        assert_eq!(emit(|o| emit_if_type(128, o)), vec![0x04, 0x80, 0x01]);
    }

    #[test]
    fn op_locals_run() {
        assert_eq!(
            emit(|o| emit_locals_run(3, ValType::I32, o)),
            vec![0x03, 0x7F]
        );
    }

    #[test]
    fn small_module_with_one_export_is_well_formed() {
        // (module
        //   (type (func (result i32)))
        //   (func (export "answer") (result i32) i32.const 42)
        // )
        let mut m = WasmModule::new();
        let ty = m.add_type(FuncType {
            params: vec![],
            results: vec![ValType::I32],
        });
        let mut body = Vec::new();
        // locals = 0
        encode_u32(0, &mut body);
        // i32.const 42 ; end
        body.push(0x41);
        encode_s32(42, &mut body);
        body.push(0x0B); // end
        let fn_idx = m.add_func(ty, body);
        m.add_func_export("answer", fn_idx);
        let bytes = m.finish();
        // Sanity-check the structure: magic + type section (1) +
        // function section (3) + export section (7) + code section (10).
        assert_eq!(&bytes[0..8], b"\0asm\x01\0\0\0");
        // Walk section IDs after the header.
        let mut p = 8;
        let mut section_ids = Vec::new();
        while p < bytes.len() {
            let id = bytes[p];
            section_ids.push(id);
            p += 1;
            // Decode size LEB128.
            let mut size = 0u32;
            let mut shift = 0;
            loop {
                let b = bytes[p];
                p += 1;
                size |= ((b & 0x7F) as u32) << shift;
                if b & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            p += size as usize;
        }
        assert_eq!(section_ids, vec![1, 3, 7, 10]);
        assert_eq!(p, bytes.len());
    }

    /// End-to-end: build `and_mask(a, b) = a & b; return` using both the
    /// `WasmModule` builder AND the new instruction emitters, then assert
    /// the magic + section IDs and validate with `wasmparser`.
    #[test]
    fn end_to_end_and_mask_module_parses() {
        let mut m = WasmModule::new();
        let ty = m.add_type(FuncType {
            params: vec![ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        });
        let mut body = Vec::new();
        encode_u32(0, &mut body); // no extra locals beyond params
        emit_local_get(0, &mut body);
        emit_local_get(1, &mut body);
        emit_i32_and(&mut body);
        emit_return(&mut body);
        emit_end(&mut body);
        let fn_idx = m.add_func(ty, body);
        m.add_func_export("and_mask", fn_idx);
        let bytes = m.finish();

        // Magic header.
        assert_eq!(&bytes[0..8], b"\0asm\x01\0\0\0");

        // Walk via wasmparser: collect section IDs and prove every payload
        // (including operators in the code body) parses cleanly.
        use wasmparser::{Parser, Payload};
        let mut section_ids = Vec::new();
        let mut saw_code = false;
        let mut saw_export = false;
        for payload in Parser::new(0).parse_all(&bytes) {
            match payload.expect("wasmparser must parse our bytes") {
                Payload::TypeSection(_) => section_ids.push(1),
                Payload::FunctionSection(_) => section_ids.push(3),
                Payload::ExportSection(r) => {
                    section_ids.push(7);
                    for ex in r {
                        assert_eq!(ex.expect("export decodes").name, "and_mask");
                    }
                    saw_export = true;
                }
                Payload::CodeSectionStart { .. } => section_ids.push(10),
                Payload::CodeSectionEntry(body) => {
                    for op in body.get_operators_reader().expect("ops reader") {
                        op.expect("each op decodes");
                    }
                    saw_code = true;
                }
                _ => {}
            }
        }
        assert_eq!(section_ids, vec![1, 3, 7, 10]);
        assert!(saw_code && saw_export);
    }
}
