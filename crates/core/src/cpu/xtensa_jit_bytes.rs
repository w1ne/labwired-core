// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa JIT runtime-agnostic emit artifacts (#124 Phase 4).
//!
//! This module is **always** compiled — independent of the `jit` feature.
//! Reason: the browser-side JIT prototype lives in `labwired-wasm`, which
//! doesn't (and can't) enable `jit` (wasmtime doesn't build for
//! wasm32-unknown-unknown). But it still needs the compiled wasm bytes +
//! the architectural constants (block PC bounds, instruction count, L32R
//! literal address). So we keep emit output here, outside the JIT feature
//! gate, while the runtime backends — wasmtime native vs `js_sys` in the
//! browser — stay gated to their respective build configurations.
//!
//! The wasm bytes are pre-compiled at crate build time by `build.rs` from
//! the canonical WAT source at `src/cpu/xtensa_jit/hot_bb.wat`. Both
//! backends consume bit-identical bytes; emit decoupling is exactly that
//! shared byte stream.

/// PC of the JITed hot multi-instruction BB (`call_start_cpu0` delay loop).
/// Mirrors `xtensa_jit::bb_multi::HOT_BB_PC`; duplicated here so the
/// browser path can reference it without requiring the `jit` feature.
pub const HOT_BB_PC: u32 = 0x400829cc;
/// First PC after the JITed range; the interpreter resumes here at
/// `callx8`.  Mirrors `xtensa_jit::bb_multi::HOT_BB_END`.
pub const HOT_BB_END: u32 = 0x400829e4;
/// Number of Xtensa instructions executed by the JITed body.
pub const HOT_BB_INSTR_COUNT: u32 = 8;
/// L32R literal address read by the block. Mirrors
/// `xtensa_jit::bb_multi::HOT_BB_L32R_ADDR`.
pub const HOT_BB_L32R_ADDR: u32 = 0x4008_0534;

/// Side-exit: block executed cleanly to the terminator.
pub const EXIT_FALL_THROUGH: i32 = 0;
/// Side-exit: host `read_u8` import signalled a bus error.
pub const EXIT_HOST_BUS_ERROR: i32 = 5;

/// Hot-block wasm module bytes, pre-compiled at crate build time. Backends
/// instantiate via either `wasmtime::Module::new` (native) or
/// `js_sys::WebAssembly::Module::new` (browser) with these exact bytes.
pub const HOT_BB_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/xtensa_jit_hot_bb.wasm"));

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke: the baked artifact looks like a wasm module (magic + version).
    /// Catches build.rs regressions that would leave the OUT_DIR file empty
    /// or otherwise malformed.
    #[test]
    fn baked_wasm_has_magic_header() {
        assert!(
            HOT_BB_WASM.len() > 32,
            "hot_bb.wasm suspiciously small: {} bytes",
            HOT_BB_WASM.len()
        );
        // \0asm magic
        assert_eq!(&HOT_BB_WASM[0..4], b"\0asm");
        // wasm version 1
        assert_eq!(&HOT_BB_WASM[4..8], &[0x01, 0x00, 0x00, 0x00]);
    }

    /// Constants must match the JIT module's view of the block (so the
    /// browser can reference them without pulling in the `jit` feature).
    #[cfg(feature = "jit")]
    #[test]
    fn constants_agree_with_jit_module() {
        use crate::cpu::xtensa_jit::{
            HOT_BB_END as JIT_END, HOT_BB_INSTR_COUNT as JIT_N, HOT_BB_L32R_ADDR as JIT_L32R,
            HOT_BB_PC as JIT_PC,
        };
        assert_eq!(HOT_BB_PC, JIT_PC);
        assert_eq!(HOT_BB_END, JIT_END);
        assert_eq!(HOT_BB_INSTR_COUNT, JIT_N);
        assert_eq!(HOT_BB_L32R_ADDR, JIT_L32R);
    }
}
