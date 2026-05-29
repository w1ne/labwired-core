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
}
