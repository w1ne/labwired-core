// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 JIT pilot — issue #124 Phase 3.2.
//!
//! Compiles a single, hand-picked hot basic block (the inner store loop of
//! `GxEPD2_290_C90c::fillScreen` at PC `0x400d14b8`) to a WebAssembly module
//! and runs it through wasmtime in native mode. The pilot proves the
//! JIT-via-wasm architecture works on a memory-touching block and gives us
//! an honest per-block call-overhead number to inform Phase 4 scaling.
//!
//! ## Target block disassembly
//!
//! Verified against `/tmp/labwired-ereader/build/labwired-ereader.ino.elf`
//! with the PlatformIO `xtensa-esp32-elf-objdump`:
//!
//! ```text
//! 400d14b8: 00 42 92      s8i    a9, a2, 0     ; mem8[a2 + 0] = a9 & 0xFF
//! 400d14bb: b2 aa         add.n  a11, a2, a10  ; a11 = a2 + a10
//! 400d14bd: 00 4b 82      s8i    a8, a11, 0    ; mem8[a11 + 0] = a8 & 0xFF
//! 400d14c0: 22 1b         addi.n a2, a2, 1
//! 400d14c2: 33 0b         addi.n a3, a3, -1
//! 400d14c4: ff 03 56      bnez   a3, 400d14b8  ; loop while a3 != 0
//! ```
//!
//! Block range: `[0x400d14b8, 0x400d14c7)` (15 bytes, 6 instructions).
//! Reads: a2, a3, a8, a9, a10. Writes: a2, a3, a11, and two bytes of memory.
//! Terminator: conditional branch back to block start; fall-through is the
//! `retw.n` at `0x400d14c7`.
//!
//! ## Side-exit codes (per #124)
//!
//! * `0` — natural fall-through; new PC = block end (`0x400d14c7`).
//! * `1` — branch taken; new PC = block start (`0x400d14b8`). The JIT runs
//!   one pass through the block in a single wasm call; if the terminating
//!   `bnez` fires we re-enter via the outer step loop on the next iteration.
//! * `3` — store address outside the inlined DRAM range; PC reverts to
//!   block start with **no register/memory mutation** so the interpreter
//!   can faithfully re-run the block and raise the genuine bus error.
//!
//! ## Host-side store callback
//!
//! Stores are dispatched to a host import (`host.store_u8`) rather than to
//! a wasm linear memory aliased to host DRAM. Reasoning:
//!
//! 1. Linear-memory aliasing across a wasmtime sandbox boundary is
//!    expensive to set up and breaks abstraction with the `Bus` trait
//!    (peripherals, traps, observers all live on the host).
//! 2. The pilot's primary goal is to measure *call overhead*, not to
//!    optimise byte-store throughput. A host import faithfully exposes the
//!    same dispatch latency a real per-block JIT would pay for any
//!    non-trivial memory model.
//!
//! Bounds-checking still happens **inside wasm** as the spec requires —
//! the host import is only invoked once wasm has cleared both stores'
//! destination addresses against the DRAM range.

#![cfg(feature = "jit")]

use std::collections::HashMap;
use std::sync::Mutex;

use wasmtime::{Engine, Func, Instance, Module, Store, TypedFunc};

/// PC of the target basic block (see disassembly above).
pub const FILL_SCREEN_BLOCK_PC: u32 = 0x400d14b8;
/// First PC after the block — fall-through destination.
pub const FILL_SCREEN_BLOCK_END: u32 = 0x400d14c7;
/// Number of Xtensa instructions in the block (used to advance CCOUNT).
pub const FILL_SCREEN_BLOCK_INSTR_COUNT: u32 = 6;

/// DRAM bounds inlined into the compiled wasm (must mirror the
/// `configure_xtensa_esp32` peripheral map: dram = [0x3FFAE000, 0x3FFE0000),
/// sram1 = [0x3FFE0000, 0x40000000)). We use the union `[0x3FFAE000, 0x40000000)`.
const DRAM_LO: u32 = 0x3FFA_E000;
const DRAM_HI: u32 = 0x4000_0000;

/// Host-side scratch slot the wasm import drains into. Wrapped in a Mutex so
/// the closure passed to `Func::wrap` can be `Sync` without us hand-rolling
/// unsafe pointer aliasing into the wasmtime Store.
///
/// Each `run()` call clears this, the wasm guest pushes 0–2 `(addr, val)`
/// pairs into it via `host.store_u8`, and the caller drains them onto the
/// bus after wasm returns. We don't dispatch directly from inside the import
/// because the `Bus` trait object isn't `Send + Sync` and threading it
/// through wasmtime's host-function ABI would require unsafe self-borrows.
type PendingStores = Vec<(u32, u8)>;

/// Wasm function signature for the `fillScreen` block:
///   Params:  (a8, a9, a2, a3, a10) — all u32 carried as i32.
///   Returns: (exit_code, a2_new, a3_new, a11_new) — all i32.
type FillScreenParams = (i32, i32, i32, i32, i32);
type FillScreenReturns = (i32, i32, i32, i32);
type FillScreenRun = TypedFunc<FillScreenParams, FillScreenReturns>;

/// Outcome of running a compiled block: `(exit_code, new_a2, new_a3,
/// new_a11, pending_stores)`. See module docs for exit-code semantics.
type RunResult = (i32, u32, u32, u32, Vec<(u32, u8)>);

/// One compiled block instance.
///
/// We hold the `Store`, `Instance`, and the typed `run` function. The block
/// holds its own per-block scratch buffer (`pending`) so we don't pay the
/// `Vec` allocation on the hot path.
pub struct CompiledBlock {
    store: Store<()>,
    /// Typed view of the exported `run` function.
    run: FillScreenRun,
    /// Stores queued by the in-flight wasm call. Drained by `Self::run` after
    /// `run.call()` returns; the caller copies them onto the live bus.
    pending: std::sync::Arc<Mutex<PendingStores>>,
    /// Number of times this block has been invoked. Surfaced via
    /// `JitCache::hit_count` for the pilot benchmark write-up.
    pub hits: u64,
}

impl CompiledBlock {
    /// Invoke the compiled block. On success returns
    /// `(exit_code, a2_new, a3_new, a11_new, pending_stores)`. The caller
    /// must apply `pending_stores` to the bus iff `exit_code != 3`.
    #[inline]
    pub fn run(
        &mut self,
        a8: u32,
        a9: u32,
        a2: u32,
        a3: u32,
        a10: u32,
    ) -> wasmtime::Result<RunResult> {
        // Clear the scratch buffer. Lock is uncontended on this hot path —
        // wasm runs synchronously on the same thread.
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
pub struct JitCache {
    engine: Engine,
    compiled: HashMap<u32, CompiledBlock>,
}

impl Default for JitCache {
    fn default() -> Self {
        Self::new()
    }
}

impl JitCache {
    pub fn new() -> Self {
        // Default wasmtime config: cranelift compiler, native target. We
        // disable the epoch interruption and fuel features (not used) to
        // keep call overhead minimal.
        let engine = Engine::default();
        Self {
            engine,
            compiled: HashMap::new(),
        }
    }

    /// If `pc` is a known JIT-compilable block, return a mutable reference
    /// to its compiled form, building+caching on first sight. Returns
    /// `None` for any PC we don't know how to compile yet.
    pub fn lookup_or_install(&mut self, pc: u32) -> Option<&mut CompiledBlock> {
        if pc == FILL_SCREEN_BLOCK_PC {
            if !self.compiled.contains_key(&pc) {
                match self.build_fill_screen() {
                    Ok(cb) => {
                        self.compiled.insert(pc, cb);
                    }
                    Err(e) => {
                        // Compilation failure: log and never try this PC
                        // again in this run. The interpreter falls through
                        // and the test/bench still completes correctly.
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

    /// Total number of times any compiled block has been invoked since
    /// process start. Used by the pilot's CLI-level reporting.
    pub fn total_hits(&self) -> u64 {
        self.compiled.values().map(|cb| cb.hits).sum()
    }

    /// Build the `fillScreen` body block. See module-level docs for the
    /// disassembly and side-exit protocol.
    fn build_fill_screen(&self) -> wasmtime::Result<CompiledBlock> {
        // Hand-written WAT. wasmtime parses WAT directly via
        // `Module::new(&engine, wat_str)`. The bounds-check uses unsigned
        // comparison against the inlined DRAM range, so a2/a11 below
        // 0x80000000 (their natural i32 sign bit clear range) match the
        // u32 comparison we'd do in Rust.
        //
        // Returns four i32s: (exit_code, a2_new, a3_new, a11_new). Stores
        // are queued via the `host.store_u8` import; wasm only calls it
        // after the bounds check passes for BOTH addresses, so an exit=3
        // path leaves zero side effects (no queued stores, no register
        // mutation observable to the caller).
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

        // Shared scratch buffer between the host closure and `CompiledBlock`.
        let pending: std::sync::Arc<Mutex<PendingStores>> =
            std::sync::Arc::new(Mutex::new(Vec::with_capacity(2)));
        let pending_for_import = pending.clone();

        let store_u8: Func = Func::wrap(&mut store, move |addr: i32, val: i32| {
            // Wasm sends us i32; reinterpret as u32 unsigned address + u8 byte.
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

#[cfg(test)]
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
        // Pick addresses inside DRAM. a2 = 0x3FFB_0000, a10 = 0x100,
        // so a11 = 0x3FFB_0100. Both inside [0x3FFAE000, 0x40000000).
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
        // a2 inside DRAM, a10 huge so a2+a10 overflows above DRAM_HI.
        let (exit, _a2, _a3, _a11, pending) = block.run(0, 0, 0x3FFB_0000, 1, 0x1000_0000).unwrap();
        assert_eq!(exit, 3);
        assert!(pending.is_empty());
    }

    /// `lookup_or_install` returns None for unknown PCs (interpreter falls
    /// through to existing path).
    #[test]
    fn unknown_pc_returns_none() {
        let mut cache = JitCache::new();
        assert!(cache.lookup_or_install(0x4000_1234).is_none());
    }
}
