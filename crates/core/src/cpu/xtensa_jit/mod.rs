// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT pilot — issue #124.
//!
//! Owns the JIT pipeline split into:
//!   * [`emit_core`] — runtime-agnostic BB walker + emit core. Always
//!     compiled (no wasmtime dep). Both the native and browser
//!     backends call into it to produce wasm bytes.
//!   * [`bb_multi`] — wasmtime adapter for the multi-op hot block.
//!     Feature-gated on `jit` (only built when wasmtime is in the
//!     dep graph).
//!   * [`windowed_call`] — wasmtime adapter for the CALL8 windowed
//!     block. Also `jit`-gated.
//!
//! The Phase 4.1 refactor pulled the walker + emit allowlist out of
//! `bb_multi` into [`emit_core`] so the browser-side prototype in
//! `labwired-wasm` (which can't enable `jit` — wasmtime doesn't compile
//! for `wasm32-unknown-unknown`) can share the same code.
//!
//! ## Side-exit codes (per #124)
//!
//! See [`crate::cpu::xtensa_jit_bytes`] and [`emit_core::SideExitReason`]
//! for the wire-level vocabulary both backends agree on.
//!
//! ## Host-side store callback
//!
//! Stores / loads are dispatched to host imports rather than to a wasm
//! linear memory aliased to host DRAM. Reasoning: linear-memory aliasing
//! across the wasmtime sandbox boundary is expensive to set up and
//! breaks abstraction with the `Bus` trait (peripherals, traps,
//! observers all live on the host). The host import is only invoked
//! once wasm has cleared the store/load address against the inlined
//! DRAM range.

// `emit_core` is always compiled — it has no wasmtime dep and is shared
// with the browser-side JIT in `labwired-wasm`.
pub mod emit_core;

// Wasmtime-backed adapters live behind the `jit` feature so the browser
// build doesn't have to drag wasmtime into its dep tree.
#[cfg(feature = "jit")]
mod bb_multi;
#[cfg(feature = "jit")]
mod windowed_call;

#[cfg(feature = "jit")]
pub use bb_multi::{
    walk_bb, DecodedOp, MultiOpBlock, MultiOpResult, EXIT_FALL_THROUGH as MULTI_EXIT_FALL_THROUGH,
    EXIT_HOST_BUS_ERROR as MULTI_EXIT_HOST_BUS_ERROR, HOT_BB_END, HOT_BB_INSTR_COUNT,
    HOT_BB_L32R_ADDR, HOT_BB_PC,
};
#[cfg(feature = "jit")]
pub use windowed_call::{
    WindowedCallBlock, WindowedCallResult, EXIT_TAKEN as WINDOWED_EXIT_TAKEN, EXIT_WINDOWED_REFUSE,
    LOOPV_CALL8_INSTR_COUNT, LOOPV_CALL8_NEXT_PC, LOOPV_CALL8_PC, LOOPV_CALL8_TARGET,
};

#[cfg(feature = "jit")]
use std::collections::HashMap;
#[cfg(feature = "jit")]
use std::sync::Mutex;

#[cfg(feature = "jit")]
use wasmtime::{Engine, Func, Instance, Module, Store, TypedFunc};

// ── Phase 3.2 pilot: hand-crafted fillScreen body block ───────────────
//
// Kept for back-compat with `try_jit_step`'s fillScreen branch +
// existing unit tests. Not exercised by the multi-op JIT path
// (`HOT_BB_PC` short-circuits first in `xtensa_lx7::try_jit_step`).
// All of this is `jit`-gated because it pulls in wasmtime directly.

/// PC of the target basic block (see disassembly in `bb_multi`).
#[cfg(feature = "jit")]
pub const FILL_SCREEN_BLOCK_PC: u32 = 0x400d14b8;
/// First PC after the block — fall-through destination.
#[cfg(feature = "jit")]
pub const FILL_SCREEN_BLOCK_END: u32 = 0x400d14c7;
/// Number of Xtensa instructions in the block (used to advance CCOUNT).
#[cfg(feature = "jit")]
pub const FILL_SCREEN_BLOCK_INSTR_COUNT: u32 = 6;

/// DRAM bounds inlined into the compiled wasm (must mirror the
/// `configure_xtensa_esp32` peripheral map: dram = [0x3FFAE000, 0x3FFE0000),
/// sram1 = [0x3FFE0000, 0x40000000)). We use the union `[0x3FFAE000, 0x40000000)`.
#[cfg(feature = "jit")]
const DRAM_LO: u32 = 0x3FFA_E000;
#[cfg(feature = "jit")]
const DRAM_HI: u32 = 0x4000_0000;

/// Host-side scratch slot the wasm import drains into. Wrapped in a Mutex so
/// the closure passed to `Func::wrap` can be `Sync` without us hand-rolling
/// unsafe pointer aliasing into the wasmtime Store.
#[cfg(feature = "jit")]
type PendingStores = Vec<(u32, u8)>;

/// Wasm function signature for the `fillScreen` block:
///   Params:  (a8, a9, a2, a3, a10) — all u32 carried as i32.
///   Returns: (exit_code, a2_new, a3_new, a11_new) — all i32.
#[cfg(feature = "jit")]
type FillScreenParams = (i32, i32, i32, i32, i32);
#[cfg(feature = "jit")]
type FillScreenReturns = (i32, i32, i32, i32);
#[cfg(feature = "jit")]
type FillScreenRun = TypedFunc<FillScreenParams, FillScreenReturns>;

/// Outcome of running a compiled block: `(exit_code, new_a2, new_a3,
/// new_a11, pending_stores)`.
#[cfg(feature = "jit")]
type RunResult = (i32, u32, u32, u32, Vec<(u32, u8)>);

/// One compiled block instance.
#[cfg(feature = "jit")]
pub struct CompiledBlock {
    store: Store<()>,
    run: FillScreenRun,
    pending: std::sync::Arc<Mutex<PendingStores>>,
    pub hits: u64,
}

#[cfg(feature = "jit")]
impl CompiledBlock {
    /// Invoke the compiled block. On success returns
    /// `(exit_code, a2_new, a3_new, a11_new, pending_stores)`.
    #[inline]
    pub fn run(
        &mut self,
        a8: u32,
        a9: u32,
        a2: u32,
        a3: u32,
        a10: u32,
    ) -> wasmtime::Result<RunResult> {
        self.pending.lock().unwrap().clear();
        let (exit, a2_n, a3_n, a11_n) = self.run.call(
            &mut self.store,
            (a8 as i32, a9 as i32, a2 as i32, a3 as i32, a10 as i32),
        )?;
        let pend = std::mem::take(&mut *self.pending.lock().unwrap());
        self.hits += 1;
        Ok((exit, a2_n as u32, a3_n as u32, a11_n as u32, pend))
    }
}

/// Cache of compiled blocks keyed by start PC. Re-uses the shared wasmtime
/// `Engine` across all blocks so module compilation amortises.
#[cfg(feature = "jit")]
pub struct JitCache {
    engine: Engine,
    compiled: HashMap<u32, CompiledBlock>,
    /// Phase 3.6.2: windowed CALL8 block for `_Z4loopv` (PC 0x400d4a99).
    pub loopv_call8: Option<Box<WindowedCallBlock>>,
    /// Phase 3.6.2 instrumentation: count windowed-call refusals.
    pub windowed_refusals: u64,
    /// Phase 3.6.3: multi-op block for the call_start_cpu0 hot loop at
    /// PC 0x400829cc.
    pub hot_bb: Option<Box<MultiOpBlock>>,
    /// Phase 3.6.3 instrumentation: count multi-op refusals.
    pub multi_op_refusals: u64,
}

#[cfg(feature = "jit")]
impl Default for JitCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "jit")]
impl JitCache {
    pub fn new() -> Self {
        let engine = Engine::default();
        Self {
            engine,
            compiled: HashMap::new(),
            loopv_call8: None,
            windowed_refusals: 0,
            hot_bb: None,
            multi_op_refusals: 0,
        }
    }

    /// If `pc` is a known JIT-compilable block, return a mutable reference
    /// to its compiled form, building+caching on first sight.
    pub fn lookup_or_install(&mut self, pc: u32) -> Option<&mut CompiledBlock> {
        if pc == FILL_SCREEN_BLOCK_PC {
            if !self.compiled.contains_key(&pc) {
                match self.build_fill_screen() {
                    Ok(cb) => {
                        self.compiled.insert(pc, cb);
                    }
                    Err(e) => {
                        tracing::warn!(target: "labwired-core::jit",
                            "JIT compile failed for pc=0x{pc:08x}: {e:#}. \
                             Falling back to interpreter for this PC.");
                        return None;
                    }
                }
            }
            return self.compiled.get_mut(&pc);
        }
        None
    }

    /// Lazily compile + return the LOOPV CALL8 block.
    pub fn lookup_or_install_windowed(&mut self, pc: u32) -> Option<&mut WindowedCallBlock> {
        if pc != LOOPV_CALL8_PC {
            return None;
        }
        if self.loopv_call8.is_none() {
            match WindowedCallBlock::build_loopv(&self.engine) {
                Ok(b) => self.loopv_call8 = Some(Box::new(b)),
                Err(e) => {
                    tracing::warn!(target: "labwired-core::jit",
                        "windowed CALL8 JIT compile failed for pc=0x{pc:08x}: {e:#}. \
                         Falling back to interpreter for this PC.");
                    return None;
                }
            }
        }
        self.loopv_call8.as_deref_mut()
    }

    /// Lazily compile + return the multi-op block for `call_start_cpu0`.
    pub fn lookup_or_install_multi_op(&mut self, pc: u32) -> Option<&mut MultiOpBlock> {
        if pc != HOT_BB_PC {
            return None;
        }
        if self.hot_bb.is_none() {
            match MultiOpBlock::build_hot_bb(&self.engine) {
                Ok(b) => self.hot_bb = Some(Box::new(b)),
                Err(e) => {
                    tracing::warn!(target: "labwired-core::jit",
                        "multi-op JIT compile failed for pc=0x{pc:08x}: {e:#}. \
                         Falling back to interpreter for this PC.");
                    return None;
                }
            }
        }
        self.hot_bb.as_deref_mut()
    }

    /// Total number of times any compiled block has been invoked since
    /// process start.
    pub fn total_hits(&self) -> u64 {
        let fillscreen_hits: u64 = self.compiled.values().map(|cb| cb.hits).sum();
        let windowed_hits = self.loopv_call8.as_ref().map(|b| b.hits).unwrap_or(0);
        let multi_op_hits = self.hot_bb.as_ref().map(|b| b.hits).unwrap_or(0);
        fillscreen_hits + windowed_hits + multi_op_hits
    }

    /// Build the `fillScreen` body block.
    fn build_fill_screen(&self) -> wasmtime::Result<CompiledBlock> {
        let wat = format!(
            r#"(module
  (import "host" "store_u8" (func $store (param i32 i32)))
  (func (export "run")
        (param $a8 i32) (param $a9 i32) (param $a2 i32) (param $a3 i32) (param $a10 i32)
        (result i32 i32 i32 i32)
    (local $a11 i32)
    (local $exit i32)

    ;; Pre-compute a11 (used in second store + as return value).
    (local.set $a11 (i32.add (local.get $a2) (local.get $a10)))

    ;; Bounds check #1: a2 in [DRAM_LO, DRAM_HI)
    (if (i32.or
          (i32.lt_u (local.get $a2) (i32.const {dram_lo}))
          (i32.ge_u (local.get $a2) (i32.const {dram_hi})))
      (then
        (return (i32.const 3) (local.get $a2) (local.get $a3) (local.get $a11))))

    ;; Bounds check #2: a11 in [DRAM_LO, DRAM_HI)
    (if (i32.or
          (i32.lt_u (local.get $a11) (i32.const {dram_lo}))
          (i32.ge_u (local.get $a11) (i32.const {dram_hi})))
      (then
        (return (i32.const 3) (local.get $a2) (local.get $a3) (local.get $a11))))

    ;; Both addresses cleared — dispatch the two byte stores to the host.
    (call $store (local.get $a2)  (i32.and (local.get $a9) (i32.const 0xFF)))
    (call $store (local.get $a11) (i32.and (local.get $a8) (i32.const 0xFF)))

    ;; addi.n a2, a2, 1
    (local.set $a2 (i32.add (local.get $a2) (i32.const 1)))
    ;; addi.n a3, a3, -1
    (local.set $a3 (i32.sub (local.get $a3) (i32.const 1)))

    ;; bnez a3, block_start  -> exit=1 means "take branch (PC = block_start)",
    ;; exit=0 means "fall through (PC = block_end)".
    (if (result i32)
      (i32.eqz (local.get $a3))
      (then (local.set $exit (i32.const 0)) (i32.const 0))
      (else (local.set $exit (i32.const 1)) (i32.const 0)))
    drop

    (local.get $exit)
    (local.get $a2)
    (local.get $a3)
    (local.get $a11)
  )
)
"#,
            dram_lo = DRAM_LO,
            dram_hi = DRAM_HI,
        );

        let module = Module::new(&self.engine, wat)?;
        let mut store: Store<()> = Store::new(&self.engine, ());

        let pending: std::sync::Arc<Mutex<PendingStores>> =
            std::sync::Arc::new(Mutex::new(Vec::with_capacity(2)));
        let pending_for_import = pending.clone();

        let store_u8: Func = Func::wrap(&mut store, move |addr: i32, val: i32| {
            pending_for_import
                .lock()
                .unwrap()
                .push((addr as u32, (val & 0xFF) as u8));
        });

        let instance = Instance::new(&mut store, &module, &[store_u8.into()])?;
        let run =
            instance.get_typed_func::<FillScreenParams, FillScreenReturns>(&mut store, "run")?;

        Ok(CompiledBlock {
            store,
            run,
            pending,
            hits: 0,
        })
    }
}

#[cfg(all(test, feature = "jit"))]
mod tests {
    use super::*;

    /// Build the module, run one iteration with `a3=1` so the loop exits via
    /// fall-through, and verify both stores were queued correctly.
    #[test]
    fn fill_screen_single_iteration_fallthrough() {
        let mut cache = JitCache::new();
        let block = cache
            .lookup_or_install(FILL_SCREEN_BLOCK_PC)
            .expect("fillScreen JIT must compile");
        let (exit, a2_n, a3_n, a11_n, pending) = block
            .run(
                /*a8*/ 0xAA,
                /*a9*/ 0xBB,
                /*a2*/ 0x3FFB_0000,
                /*a3*/ 1,
                /*a10*/ 0x100,
            )
            .expect("wasm call ok");
        assert_eq!(exit, 0, "a3 decremented to 0 -> fall-through");
        assert_eq!(a2_n, 0x3FFB_0001);
        assert_eq!(a3_n, 0);
        assert_eq!(a11_n, 0x3FFB_0100);
        assert_eq!(
            pending,
            vec![(0x3FFB_0000u32, 0xBBu8), (0x3FFB_0100u32, 0xAAu8)]
        );
        assert_eq!(block.hits, 1);
    }

    /// `a3 > 1` after decrement -> branch-taken side-exit (code 1).
    #[test]
    fn fill_screen_branch_taken() {
        let mut cache = JitCache::new();
        let block = cache.lookup_or_install(FILL_SCREEN_BLOCK_PC).unwrap();
        let (exit, a2_n, a3_n, _a11, pending) = block.run(0, 0, 0x3FFB_0000, 5, 0x100).unwrap();
        assert_eq!(exit, 1, "a3=4 after decrement -> bnez taken");
        assert_eq!(a2_n, 0x3FFB_0001);
        assert_eq!(a3_n, 4);
        assert_eq!(pending.len(), 2);
    }

    /// First store address outside DRAM -> exit=3, no stores queued, no
    /// register mutation visible.
    #[test]
    fn fill_screen_oor_first_store() {
        let mut cache = JitCache::new();
        let block = cache.lookup_or_install(FILL_SCREEN_BLOCK_PC).unwrap();
        let (exit, a2_n, a3_n, _a11, pending) = block
            .run(0, 0, /*a2 below DRAM_LO*/ 0x3000_0000, 1, 0x100)
            .unwrap();
        assert_eq!(exit, 3);
        assert_eq!(a2_n, 0x3000_0000, "a2 must be untouched on side-exit");
        assert_eq!(a3_n, 1, "a3 must be untouched on side-exit");
        assert!(pending.is_empty(), "no stores must be queued on OOR");
    }

    /// Second store (a11 = a2 + a10) lands outside DRAM even though a2 is in
    /// range -> still exit=3, still no stores.
    #[test]
    fn fill_screen_oor_second_store() {
        let mut cache = JitCache::new();
        let block = cache.lookup_or_install(FILL_SCREEN_BLOCK_PC).unwrap();
        let (exit, _a2, _a3, _a11, pending) = block.run(0, 0, 0x3FFB_0000, 1, 0x1000_0000).unwrap();
        assert_eq!(exit, 3);
        assert!(pending.is_empty());
    }

    /// `lookup_or_install` returns None for unknown PCs.
    #[test]
    fn unknown_pc_returns_none() {
        let mut cache = JitCache::new();
        assert!(cache.lookup_or_install(0x4000_1234).is_none());
    }
}
