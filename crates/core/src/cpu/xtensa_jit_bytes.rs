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

/// PC of the actually-hottest steady-state block: `loopTask` polling loop
/// (Arduino-ESP32 main scheduler). Decoded as
/// `L32R → L8UI → BEQZ → CALL8 → L32R → BEQZ → J`. Phase 4.3.5 JIT-compiles
/// the **prefix** (L32R → L8UI → BEQZ), ending at the first BEQZ side-exit.
/// Full coverage needs CALL8/RETW (Phase 4.4).
pub const LOOPTASK_PC: u32 = 0x400d_4a8d;
/// Number of Xtensa instructions in the loopTask prefix (L32R, L8UI, BEQZ).
pub const LOOPTASK_PREFIX_INSTR_COUNT: u32 = 3;
/// First PC after the loopTask prefix — `LOOPTASK_PC + 3 + 3 + 3` (three
/// 3-byte instructions). Used as the fall-through PC when the BEQZ is not
/// taken; the interpreter resumes from here.
pub const LOOPTASK_PREFIX_END: u32 = LOOPTASK_PC + 9;

/// PC of the loopTask **tail** block (Phase 4.4): the `L32R → BEQZ` pair
/// at `_Z8loopTaskPv + 0x18`, immediately following the `CALL8 _Z4loopv`
/// return. Decoded as:
/// ```text
///   400d4a9c: l32r  a8, &serialEventRun
///   400d4a9f: beqz  a8, 400d4a8d        ; loop back to loopTask top
///   400d4aa2: call8 serialEventRun       ; terminator (interp)
///   400d4aa5: j     400d4a8d             ; terminator (interp)
/// ```
/// The JIT covers the L32R + BEQZ pair (BEQZ is the terminator). In
/// steady-state polling (`serialEventRun == NULL`, the default at boot)
/// BEQZ is taken every iteration and the dispatcher loops straight back
/// to [`LOOPTASK_PC`], avoiding both `CALL8 serialEventRun` and the
/// unconditional `J`. When the literal is non-zero the JIT falls through
/// to `0x400d4aa2` (the CALL8) and the interpreter handles the rest.
pub const LOOPTASK_TAIL_PC: u32 = 0x400d_4a9c;
/// Number of Xtensa instructions in the loopTask tail block. L32R + BEQZ.
pub const LOOPTASK_TAIL_INSTR_COUNT: u32 = 2;
/// First PC after the loopTask tail block — fall-through resume PC when
/// BEQZ is not taken (i.e. `serialEventRun != NULL`). The interpreter
/// resumes at the `CALL8 serialEventRun`.
pub const LOOPTASK_TAIL_END: u32 = LOOPTASK_TAIL_PC + 6;

/// Side-exit: block executed cleanly to the terminator.
pub const EXIT_FALL_THROUGH: i32 = 0;
/// Side-exit: a conditional or unconditional branch terminated the BB
/// with the taken path. The wasm body has populated the result tuple's
/// PC slot with the branch target; the interpreter resumes execution
/// from there. Wire code consumed by both backends (Phase 4.3, #124).
pub const EXIT_BRANCH_TAKEN: i32 = 1;
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
