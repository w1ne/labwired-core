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
/// Side-exit: the block's conditional branch evaluated true and the caller
/// must continue at the target PC returned alongside this code. Value 1
/// matches the pre-existing `fillScreen` / windowed-CALL8 "taken" wire
/// code so the three emitters agree on the vocabulary.
pub const EXIT_BRANCH_TAKEN: i32 = 1;
/// Side-exit: an unconditional `J` retired and the caller must continue at
/// the target PC. Distinct from [`EXIT_BRANCH_TAKEN`] because the
/// interpreter's `J` arm does **not** set `Cpu::branched` (only
/// `branch()`-family arms do), and `branched` feeds the zero-overhead-loop
/// post-instruction check: a `J` whose destination is `LEND` must still
/// loop back when its fall-through also reaches `LEND`. Value 2 is unused
/// by the hot block (0/5) and fillScreen (0/1/3/5) vocabularies.
pub const EXIT_JUMP_TAKEN: i32 = 2;
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

    /// The JIT module must not hold its own copy of these constants.
    ///
    /// This was `#[cfg(feature = "jit")] constants_agree_with_jit_module`,
    /// which never ran in a default build and, since `bb_multi.rs` began
    /// re-exporting these very items, compared each constant with itself. The
    /// real property is "one home", and it is checkable from source without the
    /// feature: `bb_multi.rs` may re-export the constants but must not declare
    /// them.
    #[test]
    fn jit_module_reexports_the_constants_and_declares_none() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cpu/xtensa_jit/bb_multi.rs");
        let src = std::fs::read_to_string(&path).expect("read bb_multi.rs");
        for name in [
            "HOT_BB_PC",
            "HOT_BB_END",
            "HOT_BB_INSTR_COUNT",
            "HOT_BB_L32R_ADDR",
        ] {
            assert!(
                src.contains(name),
                "{name} no longer appears in bb_multi.rs: update this guard"
            );
            let declared = src.lines().any(|l| {
                let l = l.trim_start();
                (l.starts_with("pub const ")
                    || l.starts_with("pub(crate) const ")
                    || l.starts_with("const "))
                    && l.contains(&format!("{name}:"))
            });
            assert!(
                !declared,
                "bb_multi.rs declares its own {name}; it must re-export \
                 cpu::xtensa_jit_bytes::{name} (one home)"
            );
        }
        assert!(
            src.contains("xtensa_jit_bytes::{"),
            "bb_multi.rs no longer re-exports from xtensa_jit_bytes"
        );
    }
}
