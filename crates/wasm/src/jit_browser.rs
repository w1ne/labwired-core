// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Browser-side Xtensa JIT (#124 Phase 4).
//!
//! ## Why this exists
//!
//! The native JIT in `labwired-core::cpu::xtensa_jit` runs hot Xtensa
//! basic blocks as WebAssembly modules via wasmtime. Wasmtime doesn't
//! compile for `wasm32-unknown-unknown`, so the deployed browser sim
//! has been paying full interpreter cost for every instruction —
//! including the dominant 0x400829cc hot block (~82% of ereader work).
//!
//! Phase 4.1 split the JIT pipeline into a runtime-agnostic emit core
//! ([`emit_core::walk_and_emit`]) plus per-runtime adapters. Phase 4.2
//! (this module) is the browser adapter: it consumes [`EmittedBlock`]
//! from emit-core, hands the bytes to `js_sys::WebAssembly::{Module,
//! Instance}`, wires up the `host.read_u8` import as a wasm-bindgen
//! `Closure`, and dispatches into wasm via `Function::call3`.
//!
//! ## Cache
//!
//! Compiled blocks are keyed by `(pc, ps_bits)` in a [`HashMap`] so
//! re-entry after the first compile is just a HashMap lookup + a wasm
//! call. The native side's `JitCache` keys by `pc` only (PsBits isn't
//! consulted at the supported-opcode set today); we key by the pair
//! anyway so when Phase 4.4 starts emitting PS-dependent code
//! (CALL{n}/RETW need CALLINC) the cache is already correct.
//!
//! ## Host import surface (Phase 4.3)
//!
//! Every compiled block gets the same five-import host object, whether
//! it needs them or not (unused wasm imports are free):
//!
//! | import | signature | host behaviour |
//! |---|---|---|
//! | `host.read_u8` | `(addr) -> i32` | dequeue a staged byte; `-1` on empty |
//! | `host.read_u32` | `(addr) -> i32` | dequeue a staged word; `-1` on empty |
//! | `host.write_u8` | `(addr, val) -> i32` | queue `(addr, val)`; always `0` |
//! | `host.write_u32` | `(addr, val) -> i32` | queue `(addr, val)`; always `0` |
//! | `host.branch_target` | `(pc, off) -> i32` | `pc.wrapping_add(off)` |
//!
//! The canonical hot block uses only `read_u8`; the Phase 4.3 generic
//! ABI (`BlockAbi::Lx7Generic`, `run(a0..a15) -> (exit, target,
//! r0..r15)`) uses all five. Loads are pre-read through the live `Bus`
//! by the dispatcher (which resolves each `LoadReq`'s `base + imm`
//! against the register file), staged via
//! [`BrowserCompiledBlock::stage_reads`], and dequeued in order. Stores
//! are queued by the import and committed by the dispatcher after a
//! clean exit. Any `-1` from a read import side-exits with
//! [`EXIT_HOST_BUS_ERROR`]; the dispatcher treats that as a refusal.
//!
//! Generic blocks are additionally refused while a zero-overhead loop is
//! armed (`LCOUNT > 0`): the interpreter applies the `LEND` loop-back
//! check after every instruction, which a multi-instruction block cannot
//! replay. The interpreter runs the region instead; the hot block (no
//! control transfers) keeps its fast path.
//!
//! ## Dispatch path
//!
//! [`try_browser_jit_step`] is the entry point. Given the current CPU,
//! the bus, and the cache:
//!   1. Read PS bits (today informational, see [`PsBits`]).
//!   2. Look up `(pc, ps_bits)` in the cache.
//!   3. On miss: ask the bus for an IRAM slice covering `pc` via
//!      [`Bus::fetch_slice`] (#119 Phase 1.2), run
//!      [`emit_core::walk_and_emit`] over it, install the resulting
//!      block into the cache.
//!   4. Dispatch on [`BlockAbi`]: the hot block marshals
//!      `(a3, a5, l32r)`; generic blocks marshal the whole 16-register
//!      file plus their load/store manifests.
//!   5. Commit registers/PC/CCOUNT on fall-through, branch-taken, or
//!      jump-taken. Conditional branches come back as `EXIT_BRANCH_TAKEN`
//!      (PC = target, `branched = true`); `J` comes back as
//!      `EXIT_JUMP_TAKEN` (PC = target, `branched = false`, matching the
//!      interpreter's `J` arm). Commit queued stores through the `Bus`
//!      first and invalidate the CPU's caches for each written address
//!      (self-modifying-code safety, same hook the interpreter's `S*I`
//!      arms use).

use js_sys::{Array, Function, Object, Reflect, Uint8Array, WebAssembly};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::{vfp_binop, VfpBinOp};
use labwired_core::cpu::jit_framework::cortex_m::emit::{
    FAULT_PC_SLOT, FAULT_RETIRED_SLOT, IT_STATE_SLOT, NEXT_PC_SLOT, RES_FLAG_SLOT,
    WIRE_CHAIN_DYNAMIC, WIRE_FALL_THROUGH, WIRE_MEM_FAULT, WIRE_UNSUPPORTED,
};
use labwired_core::cpu::jit_framework::cortex_m::host::{pack_regs, unpack_regs};
use labwired_core::cpu::jit_framework::cortex_m::CortexMFrontend;
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::xtensa_jit::emit_core::{
    self, BlockAbi, EmitError, EmittedBlock, MemWidth, PsBits,
};
use labwired_core::cpu::xtensa_jit_bytes::{
    EXIT_BRANCH_TAKEN, EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, EXIT_JUMP_TAKEN, HOT_BB_L32R_ADDR,
};
use labwired_core::cpu::xtensa_sr::{CCOUNT, LCOUNT};
use labwired_core::cpu::{CortexM, XtensaLx7};
use labwired_core::{Bus, Cpu, SimResult, SimulationConfig, SimulationObserver};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Result of running an emitted block in the browser. Mirrors the
/// native `bb_multi::MultiOpResult` 5-tuple.
pub struct BrowserMultiOpResult {
    pub exit_code: i32,
    pub a2: u32,
    pub a6: u32,
    pub a8: u32,
    pub a10: u32,
}

/// Result of running an emitted generic block in the browser. Mirrors
/// the native `variable::VariableResult` 18-value tuple.
pub struct BrowserGenericResult {
    pub exit_code: i32,
    pub target_pc: u32,
    pub regs: [u32; 16],
}

/// Closure types for the five Phase 4.3 host imports. Kept as type
/// aliases so the struct fields and `build_imports` agree on the
/// signatures the emit-core modules declare.
type ReadClosure = Closure<dyn FnMut(i32) -> i32>;
type WriteClosure = Closure<dyn FnMut(i32, i32) -> i32>;
type BranchTargetClosure = Closure<dyn FnMut(i32, i32) -> i32>;

/// One installed block: the compiled `WebAssembly.Module`, its
/// `Instance`, the cached `run` export, and the host-side queues the
/// `host.read_*` / `host.write_*` imports use.
///
/// Drop order: Rust drops fields top-to-bottom. We list the closures
/// AFTER `run` and `_instance` so the JS-reachable imports are still
/// alive whenever `run` could conceivably be called. Once we stop
/// invoking `run` (by dropping the whole struct), the closures can be
/// torn down safely.
pub struct BrowserCompiledBlock {
    /// Exported `run` function — the wasm body of the emitted block.
    /// Cached as a `Function` so dispatch is a direct call with no
    /// `Reflect::get` per invocation.
    run: Function,
    /// Host-side queue of pre-staged load values. The `read_u8` /
    /// `read_u32` closures dequeue from this each time wasm invokes
    /// them, in manifest (execution) order. Values are carried as u32;
    /// `read_u8` masks to 8 bits so a hot-path byte stage and a generic
    /// word stage can share one queue.
    reads: Rc<RefCell<Vec<u32>>>,
    /// Host-side queue of `(addr, value)` store requests the wasm body
    /// produced. The dispatcher drains and commits these through the
    /// `Bus` after a clean exit.
    writes: Rc<RefCell<Vec<(u32, u32)>>>,
    /// emit-core's view of the block — kept for `length_in_instrs`,
    /// `end_pc`, the load/store manifests, and the side-exit reason map.
    /// Cheap to clone (a Vec of bytes + small metadata) and we only do
    /// it once at install.
    emitted: EmittedBlock,
    /// Hit counter. Surfaced as `WasmSimulator::jit_hits()` so the
    /// bench harness can confirm the JIT actually fired.
    pub hits: u64,
    /// Closures must outlive the instance: the JS-side imports table
    /// references them. Dropping one while wasm could still call back
    /// would dangle. Leading underscores: never read in Rust, only
    /// held alive.
    _read_u8_closure: ReadClosure,
    _read_u32_closure: ReadClosure,
    _write_u8_closure: WriteClosure,
    _write_u32_closure: WriteClosure,
    _branch_target_closure: BranchTargetClosure,
    /// Instance keeps the module + imports rooted. Held so `run`
    /// (which is just a JS function value pulled from
    /// `instance.exports`) stays callable.
    _instance: WebAssembly::Instance,
}

impl BrowserCompiledBlock {
    /// Compile + instantiate an emitted block, returning a ready-to-run
    /// handle.
    ///
    /// Failure modes (all surfaced as `JsValue` so the dispatcher can
    /// log and refuse):
    ///   * `WebAssembly.Module(buffer)` rejects the bytes — extremely
    ///     unlikely given emit-core's output is validated by wasmtime
    ///     on the native path.
    ///   * `WebAssembly.Instance(module, imports)` errors — typically a
    ///     mismatch in the import object shape; the construction here
    ///     is exhaustively typed.
    ///   * `instance.exports.run` is not a Function — only happens if
    ///     emit-core stops emitting the `run` export.
    pub fn compile(emitted: EmittedBlock) -> Result<Self, JsValue> {
        // 1. Wrap the wasm bytes in a Uint8Array. `copy_from` so we
        //    don't hand JS a view that aliases the Rust Vec (the Vec
        //    is owned by `emitted` which we move into the struct
        //    below; safer to copy than to reason about the aliasing).
        let buf = Uint8Array::new_with_length(emitted.wasm_bytes.len() as u32);
        buf.copy_from(&emitted.wasm_bytes);

        // 2. Compile.
        let module = WebAssembly::Module::new(&buf.into())
            .map_err(|e| JsValue::from_str(&format!("WebAssembly.Module: {e:?}")))?;

        // 3. Build the host-side queues + the JS closures. The read
        //    closures share one queue (wasm consumes staged values in
        //    call order); the write closures share the store queue.
        let reads: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::with_capacity(4)));
        let writes: Rc<RefCell<Vec<(u32, u32)>>> = Rc::new(RefCell::new(Vec::with_capacity(4)));

        let reads_for_u8 = reads.clone();
        let read_u8_closure = Closure::<dyn FnMut(i32) -> i32>::new(move |_addr: i32| -> i32 {
            // Wasm passes the address but the host pre-staged the values
            // in BB order. Empty queue ⇒ bus error; the wasm body then
            // returns EXIT_HOST_BUS_ERROR.
            let mut q = reads_for_u8.borrow_mut();
            if q.is_empty() {
                return -1;
            }
            (q.remove(0) & 0xFF) as i32
        });
        let reads_for_u32 = reads.clone();
        let read_u32_closure = Closure::<dyn FnMut(i32) -> i32>::new(move |_addr: i32| -> i32 {
            let mut q = reads_for_u32.borrow_mut();
            if q.is_empty() {
                return -1;
            }
            q.remove(0) as i32
        });
        let writes_for_u8 = writes.clone();
        let write_u8_closure =
            Closure::<dyn FnMut(i32, i32) -> i32>::new(move |addr: i32, val: i32| -> i32 {
                writes_for_u8.borrow_mut().push((addr as u32, val as u32));
                0
            });
        let writes_for_u32 = writes.clone();
        let write_u32_closure =
            Closure::<dyn FnMut(i32, i32) -> i32>::new(move |addr: i32, val: i32| -> i32 {
                writes_for_u32.borrow_mut().push((addr as u32, val as u32));
                0
            });
        // branch_target(pc, decoder_prebiased_offset): the host owns the
        // architectural taken-PC arithmetic so wasm never re-derives it.
        let branch_target_closure =
            Closure::<dyn FnMut(i32, i32) -> i32>::new(move |pc: i32, offset: i32| -> i32 {
                (pc as u32).wrapping_add(offset as u32) as i32
            });

        // 4. Build imports + instantiate.
        let imports = build_imports(
            &read_u8_closure,
            &read_u32_closure,
            &write_u8_closure,
            &write_u32_closure,
            &branch_target_closure,
        )?;
        let instance = WebAssembly::Instance::new(&module, &imports)
            .map_err(|e| JsValue::from_str(&format!("WebAssembly.Instance: {e:?}")))?;

        // 5. Pluck the `run` export. emit-core always exports `run`;
        //    if that changes, this is the one place to update.
        let exports = instance.exports();
        let run_val = Reflect::get(&exports, &JsValue::from_str("run"))
            .map_err(|e| JsValue::from_str(&format!("get exports.run: {e:?}")))?;
        let run: Function = run_val
            .dyn_into::<Function>()
            .map_err(|_| JsValue::from_str("exports.run is not a Function"))?;

        Ok(Self {
            run,
            reads,
            writes,
            emitted,
            hits: 0,
            _read_u8_closure: read_u8_closure,
            _read_u32_closure: read_u32_closure,
            _write_u8_closure: write_u8_closure,
            _write_u32_closure: write_u32_closure,
            _branch_target_closure: branch_target_closure,
            _instance: instance,
        })
    }

    /// Stage the byte values the hot block's L8UI ops will receive.
    /// Caller must supply exactly as many bytes as the block expects;
    /// extras are ignored, shortages surface as `EXIT_HOST_BUS_ERROR`
    /// inside wasm.
    pub fn stage_loads(&self, bytes: &[u8]) {
        let mut q = self.reads.borrow_mut();
        q.clear();
        q.extend(bytes.iter().map(|b| *b as u32));
    }

    /// Stage the values a generic block's [`LoadReq`] manifest resolved
    /// (one per load, in execution order). Same refusal contract as
    /// [`Self::stage_loads`].
    pub(crate) fn stage_reads(&self, values: &[u32]) {
        let mut q = self.reads.borrow_mut();
        q.clear();
        q.extend_from_slice(values);
        // Drop any store requests a previous (refused) run left queued.
        // The dispatcher only drains after a clean exit, so a stale
        // `(addr, value)` would otherwise be committed against the next
        // run's store manifest. Mirrors the native adapter's
        // `variable::VariableBlock::resolve_loads`.
        self.writes.borrow_mut().clear();
    }

    /// Drain the `(addr, value)` store requests the wasm body queued, in
    /// execution order.
    pub(crate) fn drain_writes(&self) -> Vec<(u32, u32)> {
        std::mem::take(&mut *self.writes.borrow_mut())
    }

    /// Invoke the hot-block ABI (`run(a3, a5, l32r_val)`, 5 results).
    pub fn run(
        &mut self,
        a3: u32,
        a5: u32,
        l32r_val: u32,
    ) -> Result<BrowserMultiOpResult, JsValue> {
        let result = self.run.call3(
            &JsValue::NULL,
            &JsValue::from_f64(a3 as i32 as f64),
            &JsValue::from_f64(a5 as i32 as f64),
            &JsValue::from_f64(l32r_val as i32 as f64),
        )?;
        let arr = result
            .dyn_into::<Array>()
            .map_err(|_| JsValue::from_str("wasm.run return is not an Array"))?;
        if arr.length() != 5 {
            return Err(JsValue::from_str(&format!(
                "wasm.run returned {} values; expected 5",
                arr.length()
            )));
        }
        let g = |i: u32| -> i32 { arr.get(i).as_f64().map(|f| f as i64 as i32).unwrap_or(0) };
        self.hits += 1;
        Ok(BrowserMultiOpResult {
            exit_code: g(0),
            a2: g(1) as u32,
            a6: g(2) as u32,
            a8: g(3) as u32,
            a10: g(4) as u32,
        })
    }

    /// Invoke the generic ABI (`run(a0..a15)`, 18 results:
    /// `exit, target, r0..r15`). `Function::apply` handles the 16-arg
    /// array uniformly; multi-value wasm returns arrive as a JS Array.
    pub fn run_generic(&mut self, regs: &[u32; 16]) -> Result<BrowserGenericResult, JsValue> {
        let args = Array::new_with_length(16);
        for (i, r) in regs.iter().enumerate() {
            args.set(i as u32, JsValue::from_f64(*r as i32 as f64));
        }
        let result = self.run.apply(&JsValue::NULL, &args)?;
        let arr = result
            .dyn_into::<Array>()
            .map_err(|_| JsValue::from_str("wasm.run (generic) return is not an Array"))?;
        if arr.length() != 18 {
            return Err(JsValue::from_str(&format!(
                "wasm.run (generic) returned {} values; expected 18",
                arr.length()
            )));
        }
        let g = |i: u32| -> i32 { arr.get(i).as_f64().map(|f| f as i64 as i32).unwrap_or(0) };
        let mut out = [0u32; 16];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = g(i as u32 + 2) as u32;
        }
        self.hits += 1;
        Ok(BrowserGenericResult {
            exit_code: g(0),
            target_pc: g(1) as u32,
            regs: out,
        })
    }

    /// Expose the block's emit-core metadata. Used by the dispatcher
    /// to advance PC and bump CCOUNT after a clean fall-through.
    pub fn emitted(&self) -> &EmittedBlock {
        &self.emitted
    }
}

/// Build the JS `imports` object the wasm module expects. Every module
/// gets the full Phase 4.3 host surface — the hot block imports only
/// `read_u8`, generic blocks import all five, and unused properties on
/// an import object are ignored by `WebAssembly.Instance`.
fn build_imports(
    read_u8: &ReadClosure,
    read_u32: &ReadClosure,
    write_u8: &WriteClosure,
    write_u32: &WriteClosure,
    branch_target: &BranchTargetClosure,
) -> Result<Object, JsValue> {
    let host_obj = Object::new();
    let set = |name: &str, value: &JsValue| -> Result<(), JsValue> {
        Reflect::set(&host_obj, &JsValue::from_str(name), value)
            .map(|_| ())
            .map_err(|e| JsValue::from_str(&format!("set host.{name}: {e:?}")))
    };
    set("read_u8", read_u8.as_ref().unchecked_ref())?;
    set("read_u32", read_u32.as_ref().unchecked_ref())?;
    set("write_u8", write_u8.as_ref().unchecked_ref())?;
    set("write_u32", write_u32.as_ref().unchecked_ref())?;
    set("branch_target", branch_target.as_ref().unchecked_ref())?;

    let imports = Object::new();
    Reflect::set(&imports, &JsValue::from_str("host"), &host_obj)
        .map_err(|e| JsValue::from_str(&format!("set imports.host: {e:?}")))?;
    Ok(imports)
}

/// Process-wide browser JIT cache. Keyed by `(pc, ps_bits.raw)` so a
/// future PS-aware emit (Phase 4.4) doesn't share blocks across
/// different PS contexts.
///
/// Thread-locality: the browser sim runs single-threaded on the wasm
/// main thread; `js_sys` types aren't `Send` anyway. We hold the cache
/// inside the `WasmSimulator` instance rather than as a global so each
/// simulator gets its own (lets tests / playground reset cleanly).
#[derive(Default)]
pub struct BrowserJitCache {
    /// `(pc, ps_bits.raw)` → installed block. HashMap on wasm32 is
    /// fine: median lookup is one hash + one branch and the working
    /// set is tiny (one entry per JIT-compiled BB shape — Phase 4.2
    /// scope is a single block, Phase 4.3 will grow to ~dozens).
    compiled: HashMap<(u32, u32), BrowserCompiledBlock>,
    /// PCs the walker has already refused under a given PS context.
    /// Without this the dispatcher would re-walk every refused BB on
    /// every step — which is ~99% of all step()s in steady-state
    /// ereader (BROM thunks, runtime helpers, anything with a branch
    /// in it) — and the per-walk `fetch_slice + walk_bb` cost
    /// dominates the entire dispatcher. The native JIT has the same
    /// invariant baked in via `JitCache::lookup_or_install_multi_op`
    /// returning early for `pc != HOT_BB_PC`; we generalise by
    /// memoising the refusal set per (pc, ps).
    refused: HashSet<(u32, u32)>,
    /// Count of refusals — blocks the emit-core walker rejected, or
    /// blocks that returned a host-side bus error at run time. Surfaced
    /// as `WasmSimulator::jit_refusals()` for the bench harness.
    pub refusals: u64,
    /// Total hits across all compiled blocks. Tracked as a running
    /// total so `WasmSimulator::jit_hits()` is O(1).
    total_hits: u64,
    /// Cortex-M Thumb blocks keyed by entry PC.
    compiled_thumb: HashMap<u32, CortexMBrowserBlock>,
    /// Thumb PCs emit refused (too short, unmodeled, instantiate error).
    refused_thumb: HashSet<u32>,
}

impl BrowserJitCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total number of times any compiled block has been dispatched.
    /// Mirrors `JitCache::total_hits` on the native side.
    pub fn total_hits(&self) -> u64 {
        self.total_hits
    }

    /// Number of compiled blocks currently installed, across both runtime
    /// adapters (the Xtensa and Cortex-M maps). Exposed to JS as
    /// `WasmSimulator::jit_compiled_blocks()` so the browser-layer gate can
    /// distinguish "the emit walk installed a block" from "every dispatch
    /// was refused" — `total_hits` alone cannot, since a cache that never
    /// compiled has no hits either.
    pub fn compiled_blocks(&self) -> u64 {
        (self.compiled.len() + self.compiled_thumb.len()) as u64
    }

    /// Compile an [`EmittedBlock`] and install it under `(pc, ps_bits)`.
    /// On success the block is ready for dispatch via [`Self::get_mut`].
    ///
    /// Idempotent: re-inserting the same key replaces the prior block.
    /// The dispatcher checks `get_mut` first so this path only runs on
    /// a miss.
    pub fn install_from_emitted(
        &mut self,
        pc: u32,
        ps_bits: PsBits,
        emitted: EmittedBlock,
    ) -> Result<(), BrowserInstallError> {
        let block = BrowserCompiledBlock::compile(emitted).map_err(BrowserInstallError::Js)?;
        self.compiled.insert((pc, ps_bits.raw), block);
        Ok(())
    }

    /// Walk the bus at `pc` via [`emit_core::walk_and_emit`] and
    /// install the resulting block. Mirrors
    /// `JitCache::lookup_or_install_multi_op` on the native side.
    ///
    /// The `pc_to_offset` closure maps a PC back into `bus_slice` —
    /// the canonical caller is the dispatcher below, which uses the
    /// `(start, end, slice)` triple from [`Bus::fetch_slice`].
    ///
    /// Note: the on-step dispatcher inlines `walk_and_emit` +
    /// `install_from_emitted` directly so it can drop the bus borrow
    /// before touching `self` again (the borrow checker requires it).
    /// This convenience method exists for non-dispatcher callers
    /// (tests, future tooling) that don't have the bus borrow
    /// conflict.
    #[allow(
        dead_code,
        reason = "Convenience entry point for non-dispatcher callers; on-step dispatch inlines for borrow-checker reasons"
    )]
    pub fn walk_and_install(
        &mut self,
        bus_slice: &[u8],
        pc: u32,
        pc_to_offset: impl FnMut(u32) -> Option<usize>,
        ps_bits: PsBits,
    ) -> Result<(), BrowserInstallError> {
        let emitted = emit_core::walk_and_emit(bus_slice, pc, pc_to_offset, ps_bits)?;
        self.install_from_emitted(pc, ps_bits, emitted)
    }

    /// Mutable handle to the block at `(pc, ps_bits)`, or `None` if
    /// not yet installed.
    pub fn get_mut(&mut self, pc: u32, ps_bits: PsBits) -> Option<&mut BrowserCompiledBlock> {
        self.compiled.get_mut(&(pc, ps_bits.raw))
    }

    /// Bump the running hit counter. Called by the dispatcher each
    /// time a block returns a clean fall-through. Kept separate from
    /// `BrowserCompiledBlock::hits` (per-block counter for diagnostics)
    /// so the cache-wide total is O(1) to read.
    fn bump_hit(&mut self) {
        self.total_hits = self.total_hits.saturating_add(1);
    }

    fn thumb_get_mut(&mut self, pc: u32) -> Option<&mut CortexMBrowserBlock> {
        self.compiled_thumb.get_mut(&pc)
    }
}

/// Failure surface for [`BrowserJitCache::install_from_emitted`].
/// Two variants: emit-core refused the BB (e.g. unsupported opcode),
/// or the JS-side `WebAssembly.Module`/`Instance` path errored.
#[derive(Debug)]
pub enum BrowserInstallError {
    /// emit-core refused — typically `EmitError::UnsupportedShape` for
    /// blocks the Phase 4.2 emit scope doesn't cover. Caller should
    /// bump the refusal counter and fall back to the interpreter.
    Emit(EmitError),
    /// JS-side error (compile / instantiate / export-lookup). Stringified
    /// `JsValue` so the dispatcher can log without dragging the
    /// browser console into a Display impl.
    Js(JsValue),
}

impl core::fmt::Display for BrowserInstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BrowserInstallError::Emit(e) => write!(f, "emit_core: {e}"),
            BrowserInstallError::Js(v) => write!(f, "js: {v:?}"),
        }
    }
}

impl From<EmitError> for BrowserInstallError {
    fn from(e: EmitError) -> Self {
        BrowserInstallError::Emit(e)
    }
}

/// Attempt to dispatch the current PC into the browser JIT. Returns
/// `true` if the JIT handled the step (caller does NOT call
/// `Cpu::step` for this iteration); `false` otherwise.
///
/// Mirrors `XtensaLx7::try_jit_multi_op` on the native path:
///   * Look up the bus slice covering `pc` via [`Bus::fetch_slice`].
///   * Look up `(pc, ps_bits)` in the cache; on miss, walk + install.
///   * Pre-read host-input bytes through the live bus.
///   * Stage the bytes, invoke wasm.
///   * On clean fall-through: commit registers, advance PC, bump
///     CCOUNT by `length_in_instrs - 1` (the outer loop already
///     counted one).
///
/// We don't run the IRQ / pre-fetch path here — those happen on the
/// regular interpreter step. The JIT is a fast-path; non-JIT PCs fall
/// through and the caller's loop hands them to `Cpu::step`.
pub fn try_browser_jit_step(
    cpu: &mut XtensaLx7,
    bus: &mut dyn Bus,
    cache: &mut BrowserJitCache,
) -> bool {
    let pc = cpu.pc;
    let ps_bits = PsBits::from_raw(cpu.ps.as_raw());

    // Sticky-refusal fast path: if we've already walked this (pc, ps)
    // and the emit-core rejected it, don't re-walk every single step.
    // This is the same logic as the native JIT's `pc != HOT_BB_PC`
    // early-return, generalised — see the `refused` field doc for
    // why it matters perf-wise.
    if cache.refused.contains(&(pc, ps_bits.raw)) {
        return false;
    }

    // Fast path: block already installed under (pc, ps_bits). Skip
    // straight to running it.
    let installed = cache.get_mut(pc, ps_bits).is_some();

    if !installed {
        // Cold path: ask the bus for an IRAM slice covering `pc`, run
        // emit-core, install. If the bus can't serve a slice (PC is in
        // a non-RAM peripheral, or unmapped), refuse permanently —
        // non-RAM fetches would side-effect anyway, so JIT'ing them
        // is unsafe.
        //
        // We hold the bus slice across `walk_and_emit` without
        // cloning. emit-core only reads from it and produces an
        // owned EmittedBlock; the slice can be dropped immediately
        // after.
        let install_result = match bus.fetch_slice(pc as u64) {
            Some((slice_start, slice_end, slice)) => {
                if (pc as u64) < slice_start || (pc as u64) >= slice_end {
                    cache.refused.insert((pc, ps_bits.raw));
                    return false;
                }
                let emitted_result = emit_core::walk_and_emit(
                    slice,
                    pc,
                    |q| {
                        let q = q as u64;
                        if q < slice_start || q >= slice_end {
                            return None;
                        }
                        Some((q - slice_start) as usize)
                    },
                    ps_bits,
                );
                // Drop the bus borrow before we touch `cache` again
                // (the borrow checker requires it; `bus.fetch_slice`
                // returned a `&[u8]` borrowed from the bus).
                match emitted_result {
                    Ok(emitted) => Some(emitted),
                    Err(e) => {
                        // Memoise the refusal so we don't re-walk
                        // this PC on every subsequent step.
                        cache.refused.insert((pc, ps_bits.raw));
                        let _ = e; // refusal type doesn't matter here
                        cache.refusals = cache.refusals.saturating_add(1);
                        return false;
                    }
                }
            }
            None => {
                cache.refused.insert((pc, ps_bits.raw));
                return false;
            }
        };

        let emitted = match install_result {
            Some(e) => e,
            None => return false,
        };
        match cache.install_from_emitted(pc, ps_bits, emitted) {
            Ok(()) => {}
            Err(BrowserInstallError::Js(e)) => {
                web_sys_console_warn(&format!(
                    "labwired-wasm: browser JIT install failed at pc=0x{pc:08x}: {e:?}. Falling back to interpreter."
                ));
                cache.refused.insert((pc, ps_bits.raw));
                cache.refusals = cache.refusals.saturating_add(1);
                return false;
            }
            Err(BrowserInstallError::Emit(_)) => {
                // install_from_emitted doesn't produce Emit errors;
                // this arm is defensive.
                cache.refused.insert((pc, ps_bits.raw));
                cache.refusals = cache.refusals.saturating_add(1);
                return false;
            }
        }
    }

    // Fetch the block's ABI + manifests up front. Cloning the manifests
    // (a handful of `Copy` structs) keeps the dispatch free to touch the
    // `Bus` without holding the cache borrow across the reads.
    let (abi, loads, stores, end_pc, length_in_instrs) = match cache.get_mut(pc, ps_bits) {
        Some(b) => {
            let e = b.emitted();
            (
                e.abi,
                e.loads.clone(),
                e.stores.clone(),
                e.end_pc,
                e.length_in_instrs,
            )
        }
        None => return false,
    };

    let outcome = {
        let block = match cache.get_mut(pc, ps_bits) {
            Some(b) => b,
            None => return false,
        };
        match abi {
            BlockAbi::HotBb => {
                // Canonical hot-block marshalling, unchanged since Phase
                // 4.2: two L8UI bytes from [a3, a3+1] + the L32R literal
                // at HOT_BB_L32R_ADDR. A bus error during the pre-read
                // refuses the step (no refusal counter) so the
                // interpreter can raise the genuine fault.
                let a3 = cpu.regs.read_logical(3);
                let a5 = cpu.regs.read_logical(5);
                let b0 = match bus.read_u8(a3 as u64) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let b1 = match bus.read_u8(a3.wrapping_add(1) as u64) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let l32r_val = match bus.read_u32(HOT_BB_L32R_ADDR as u64) {
                    Ok(v) => v,
                    Err(_) => return false,
                };

                block.stage_loads(&[b0, b1]);
                match block.run(a3, a5, l32r_val) {
                    Ok(res) => match res.exit_code {
                        x if x == EXIT_FALL_THROUGH => {
                            cpu.regs.write_logical(10, res.a10);
                            cpu.regs.write_logical(6, res.a6);
                            cpu.regs.write_logical(2, res.a2);
                            cpu.regs.write_logical(8, res.a8);
                            cpu.pc = end_pc;
                            bump_ccount(cpu, length_in_instrs);
                            cpu.branched = false;
                            StepOutcome::Applied
                        }
                        x if x == EXIT_HOST_BUS_ERROR => StepOutcome::Refused,
                        _ => StepOutcome::Refused,
                    },
                    Err(_) => StepOutcome::Refused,
                }
            }
            BlockAbi::Lx7Generic => {
                // Zero-overhead loops: the interpreter runs the LEND
                // loop-back check after *every* retired instruction; a
                // compiled multi-instruction block bypasses it, so while a
                // hardware loop is armed (`LCOUNT > 0`) the block would miss
                // a loop-back its last instruction should have taken. Refuse
                // the compiled path and let the interpreter run the region.
                // (The canonical hot block has no control transfer, so its
                // pre-existing path is unaffected.)
                if cpu.sr.read(LCOUNT) > 0 {
                    return false;
                }
                // Generic blocks: snapshot the whole logical file, resolve
                // every manifest load against it, stage the values in
                // execution order, run the 16-register ABI.
                let mut regs = [0u32; 16];
                for (i, slot) in regs.iter_mut().enumerate() {
                    *slot = cpu.regs.read_logical(i as u8);
                }
                let mut staged = Vec::with_capacity(loads.len());
                for req in &loads {
                    let addr = regs[req.base as usize].wrapping_add(req.imm) as u64;
                    let val = match req.width {
                        MemWidth::U8 => match bus.read_u8(addr) {
                            Ok(v) => v as u32,
                            Err(_) => return false,
                        },
                        MemWidth::U32 => match bus.read_u32(addr) {
                            Ok(v) => v,
                            Err(_) => return false,
                        },
                    };
                    staged.push(val);
                }
                block.stage_reads(&staged);

                match block.run_generic(&regs) {
                    Ok(res) => match res.exit_code {
                        x if x == EXIT_FALL_THROUGH
                            || x == EXIT_BRANCH_TAKEN
                            || x == EXIT_JUMP_TAKEN =>
                        {
                            // Commit stores before the register file,
                            // mirroring the interpreter's `S*I` order. A
                            // store fault refuses the step so the
                            // interpreter re-raises; stores already
                            // committed stay committed (a documented
                            // limitation of the "queue then commit"
                            // model, same as the native fillScreen block).
                            let writes = block.drain_writes();
                            let mut store_fault = false;
                            for (req, (addr, val)) in stores.iter().zip(writes.iter()) {
                                let write_res = match req.width {
                                    MemWidth::U8 => bus.write_u8(*addr as u64, *val as u8),
                                    MemWidth::U32 => bus.write_u32(*addr as u64, *val),
                                };
                                if write_res.is_err() {
                                    store_fault = true;
                                    break;
                                }
                                // The JIT bypasses `execute`'s S*I arms, so
                                // replicate their self-modifying-code cache
                                // invalidation via the CPU's public hook.
                                cpu.invalidate_for_data_write(*addr);
                            }
                            if store_fault {
                                return false;
                            }
                            for (i, v) in res.regs.iter().enumerate() {
                                cpu.regs.write_logical(i as u8, *v);
                            }
                            cpu.pc = if res.exit_code == EXIT_FALL_THROUGH {
                                end_pc
                            } else {
                                res.target_pc
                            };
                            bump_ccount(cpu, length_in_instrs);
                            cpu.branched = res.exit_code == EXIT_BRANCH_TAKEN;
                            StepOutcome::Applied
                        }
                        // EXIT_HOST_BUS_ERROR (host import refused) or an
                        // unknown code: no state committed.
                        _ => StepOutcome::Refused,
                    },
                    Err(e) => {
                        web_sys_console_warn(&format!(
                            "labwired-wasm: browser JIT generic run failed at pc=0x{pc:08x}: {e:?}"
                        ));
                        StepOutcome::Refused
                    }
                }
            }
        }
    };

    match outcome {
        StepOutcome::Applied => {
            cache.bump_hit();
            true
        }
        StepOutcome::Refused => {
            cache.refusals = cache.refusals.saturating_add(1);
            false
        }
    }
}

/// Disposition of one dispatch attempt, computed while the cache borrow
/// is live and applied after it drops.
enum StepOutcome {
    /// Block ran cleanly and state was committed; bump hits.
    Applied,
    /// Block declined (host bus error, wasm call error, unknown exit
    /// code); bump refusals and fall back to the interpreter.
    Refused,
}

/// CCOUNT honesty: the interpreter advances CCOUNT once per retired
/// instruction; the outer step already counted the first, so the JIT
/// adds the remaining `length_in_instrs - 1`.
#[inline]
fn bump_ccount(cpu: &mut XtensaLx7, length_in_instrs: u32) {
    if length_in_instrs > 1 {
        let cc = cpu.sr.read(CCOUNT);
        cpu.sr.write(CCOUNT, cc.wrapping_add(length_in_instrs - 1));
    }
}

/// Console warn shim — avoids pulling in `web-sys` just for `console`.
/// `console.warn` is universally available; we go through wasm-bindgen
/// js_namespace = console directly.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    pub(crate) fn web_sys_console_warn(s: &str);
}

const THUMB_MIN_PROFITABLE: u32 = 4;
const THUMB_CODE_WINDOW: usize = 4096;
/// Register file + the control slots the emitted body writes (through
/// [`IT_STATE_SLOT`]), rounded up so the IT slot is inside the read-back.
const THUMB_REG_BYTES: usize = 96;

type ThumbLoadClosure = Closure<dyn FnMut(i32, i32, i32) -> i32>;
type ThumbStoreClosure = Closure<dyn FnMut(i32, i32, i32)>;
type ThumbVfpGetClosure = Closure<dyn FnMut(i32) -> i32>;
type ThumbVfpSetClosure = Closure<dyn FnMut(i32, i32)>;
type ThumbVfpBinopClosure = Closure<dyn FnMut(i32, i32, i32) -> i32>;

struct ThumbHost {
    ram: *mut u8,
    ram_len: usize,
    fpu: *mut u32,
    /// FPSCR snapshot for this call, installed from the CPU by `run`.
    fpscr: u32,
}

struct CortexMBrowserBlock {
    run: Function,
    memory: WebAssembly::Memory,
    host: Rc<RefCell<ThumbHost>>,
    end_pc: u32,
    instr_count: u32,
    has_store: bool,
    _closures: Vec<ThumbLoadClosure>,
    _void3: Vec<ThumbStoreClosure>,
    _get: Vec<ThumbVfpGetClosure>,
    _set: Vec<ThumbVfpSetClosure>,
    _binop: Vec<ThumbVfpBinopClosure>,
    _instance: WebAssembly::Instance,
}

impl CortexMBrowserBlock {
    fn compile(
        wasm_bytes: &[u8],
        ram_host: bool,
        end_pc: u32,
        instr_count: u32,
        has_store: bool,
    ) -> Result<Self, JsValue> {
        let buf = Uint8Array::new_with_length(wasm_bytes.len() as u32);
        buf.copy_from(wasm_bytes);
        let module = WebAssembly::Module::new(&buf.into())
            .map_err(|e| JsValue::from_str(&format!("Thumb WebAssembly.Module: {e:?}")))?;

        let desc = Object::new();
        Reflect::set(&desc, &JsValue::from_str("initial"), &JsValue::from(1))?;
        let memory = WebAssembly::Memory::new(&desc)
            .map_err(|e| JsValue::from_str(&format!("Thumb Memory: {e:?}")))?;

        let host = Rc::new(RefCell::new(ThumbHost {
            ram: std::ptr::null_mut(),
            ram_len: 0,
            fpu: std::ptr::null_mut(),
            fpscr: 0,
        }));

        let mut closures: Vec<ThumbLoadClosure> = Vec::new();
        let mut void3: Vec<ThumbStoreClosure> = Vec::new();
        let mut gets: Vec<ThumbVfpGetClosure> = Vec::new();
        let mut sets: Vec<ThumbVfpSetClosure> = Vec::new();
        let mut binops: Vec<ThumbVfpBinopClosure> = Vec::new();

        let imports = Object::new();
        let regs = Object::new();
        Reflect::set(&regs, &JsValue::from_str("mem"), &memory)?;
        Reflect::set(&imports, &JsValue::from_str("regs"), &regs)?;

        if ram_host {
            let h_load = host.clone();
            let load =
                Closure::<dyn FnMut(i32, i32, i32) -> i32>::new(move |off, width, signed| {
                    thumb_host_load(&h_load.borrow(), off, width, signed)
                });
            let h_store = host.clone();
            let store = Closure::<dyn FnMut(i32, i32, i32)>::new(move |off, val, width| {
                thumb_host_store(&h_store.borrow(), off, val, width);
            });
            let h_get = host.clone();
            let vget =
                Closure::<dyn FnMut(i32) -> i32>::new(move |sn| thumb_vfp_get(&h_get.borrow(), sn));
            let h_set = host.clone();
            let vset = Closure::<dyn FnMut(i32, i32)>::new(move |sd, bits| {
                thumb_vfp_set(&h_set.borrow(), sd, bits);
            });
            let h_binop = host.clone();
            let vbinop = Closure::<dyn FnMut(i32, i32, i32) -> i32>::new(move |fop, a, b| {
                thumb_vfp_binop(&h_binop.borrow(), fop, a, b)
            });

            let ram = Object::new();
            Reflect::set(
                &ram,
                &JsValue::from_str("load"),
                load.as_ref().unchecked_ref(),
            )?;
            Reflect::set(
                &ram,
                &JsValue::from_str("store"),
                store.as_ref().unchecked_ref(),
            )?;
            let vfp = Object::new();
            Reflect::set(
                &vfp,
                &JsValue::from_str("get"),
                vget.as_ref().unchecked_ref(),
            )?;
            Reflect::set(
                &vfp,
                &JsValue::from_str("set"),
                vset.as_ref().unchecked_ref(),
            )?;
            Reflect::set(
                &vfp,
                &JsValue::from_str("binop"),
                vbinop.as_ref().unchecked_ref(),
            )?;
            Reflect::set(&imports, &JsValue::from_str("ram"), &ram)?;
            Reflect::set(&imports, &JsValue::from_str("vfp"), &vfp)?;

            closures.push(load);
            void3.push(store);
            gets.push(vget);
            sets.push(vset);
            binops.push(vbinop);
        }

        let instance = WebAssembly::Instance::new(&module, &imports)
            .map_err(|e| JsValue::from_str(&format!("Thumb Instance: {e:?}")))?;
        let exports = instance.exports();
        let run_val = Reflect::get(&exports, &JsValue::from_str("run"))?;
        let run: Function = run_val
            .dyn_into::<Function>()
            .map_err(|_| JsValue::from_str("thumb exports.run is not a Function"))?;

        Ok(Self {
            run,
            memory,
            host,
            end_pc,
            instr_count,
            has_store,
            _closures: closures,
            _void3: void3,
            _get: gets,
            _set: sets,
            _binop: binops,
            _instance: instance,
        })
    }

    /// Returns `(wire, next_pc, clear_exclusive, fault_pc, fault_retired,
    /// fault_it_state)`; the last is the pre-fault IT state the caller must
    /// reinstall before the interpreter resumes mid-IT.
    fn run(
        &mut self,
        cpu: &mut CortexM,
        ram: &mut [u8],
    ) -> Result<(i32, u32, bool, u32, u32, u8), JsValue> {
        let mut x = [0u32; 16];
        pack_regs(cpu, &mut x);
        let mut bytes = [0u8; THUMB_REG_BYTES];
        for (i, w) in x.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        mem_write(&self.memory, 0, &bytes);
        {
            let mut h = self.host.borrow_mut();
            h.ram = ram.as_mut_ptr();
            h.ram_len = ram.len();
            h.fpu = cpu.fpu_s.as_mut_ptr();
            h.fpscr = cpu.fpscr;
        }
        let result = self.run.call0(&JsValue::UNDEFINED);
        {
            let mut h = self.host.borrow_mut();
            h.ram = std::ptr::null_mut();
            h.ram_len = 0;
            h.fpu = std::ptr::null_mut();
        }
        let result = result?;
        let wire = result.as_f64().unwrap_or(0.0) as i32;
        mem_read(&self.memory, 0, &mut bytes);
        for (i, w) in x.iter_mut().enumerate() {
            *w = u32::from_le_bytes([
                bytes[i * 4],
                bytes[i * 4 + 1],
                bytes[i * 4 + 2],
                bytes[i * 4 + 3],
            ]);
        }
        unpack_regs(cpu, &x);
        let next_pc = u32::from_le_bytes([
            bytes[NEXT_PC_SLOT as usize],
            bytes[NEXT_PC_SLOT as usize + 1],
            bytes[NEXT_PC_SLOT as usize + 2],
            bytes[NEXT_PC_SLOT as usize + 3],
        ]);
        let fault_pc = u32::from_le_bytes([
            bytes[FAULT_PC_SLOT as usize],
            bytes[FAULT_PC_SLOT as usize + 1],
            bytes[FAULT_PC_SLOT as usize + 2],
            bytes[FAULT_PC_SLOT as usize + 3],
        ]);
        let fault_retired = u32::from_le_bytes([
            bytes[FAULT_RETIRED_SLOT as usize],
            bytes[FAULT_RETIRED_SLOT as usize + 1],
            bytes[FAULT_RETIRED_SLOT as usize + 2],
            bytes[FAULT_RETIRED_SLOT as usize + 3],
        ]);
        let clear_exclusive = self.has_store
            && u32::from_le_bytes([
                bytes[RES_FLAG_SLOT as usize],
                bytes[RES_FLAG_SLOT as usize + 1],
                bytes[RES_FLAG_SLOT as usize + 2],
                bytes[RES_FLAG_SLOT as usize + 3],
            ]) != 0;
        let fault_it_state = bytes[IT_STATE_SLOT as usize];
        Ok((
            wire,
            next_pc,
            clear_exclusive,
            fault_pc,
            fault_retired,
            fault_it_state,
        ))
    }
}

fn mem_write(memory: &WebAssembly::Memory, offset: u32, bytes: &[u8]) {
    let view = Uint8Array::new(&memory.buffer());
    let src = Uint8Array::new_with_length(bytes.len() as u32);
    src.copy_from(bytes);
    view.set(&src, offset);
}

fn mem_read(memory: &WebAssembly::Memory, offset: u32, bytes: &mut [u8]) {
    let view = Uint8Array::new(&memory.buffer());
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = view.get_index(offset + i as u32);
    }
}

fn thumb_host_load(host: &ThumbHost, off: i32, width: i32, signed: i32) -> i32 {
    let off = off as u32 as usize;
    let width = width as u32 as usize;
    if host.ram.is_null() || width == 0 || off.saturating_add(width) > host.ram_len {
        return 0;
    }
    unsafe {
        let p = host.ram.add(off);
        match (width, signed) {
            (1, 0) => i32::from(*p),
            (1, _) => i32::from(*p as i8),
            (2, 0) => i32::from(u16::from_le_bytes([*p, *p.add(1)])),
            (2, _) => i32::from(i16::from_le_bytes([*p, *p.add(1)])),
            (4, _) => i32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]),
            _ => 0,
        }
    }
}

fn thumb_host_store(host: &ThumbHost, off: i32, val: i32, width: i32) {
    let off = off as u32 as usize;
    let width = width as u32 as usize;
    if host.ram.is_null() || width == 0 || off.saturating_add(width) > host.ram_len {
        return;
    }
    let v = val as u32;
    unsafe {
        let p = host.ram.add(off);
        match width {
            1 => *p = v as u8,
            2 => {
                let b = (v as u16).to_le_bytes();
                *p = b[0];
                *p.add(1) = b[1];
            }
            4 => {
                let b = v.to_le_bytes();
                *p = b[0];
                *p.add(1) = b[1];
                *p.add(2) = b[2];
                *p.add(3) = b[3];
            }
            _ => {}
        }
    }
}

/// The two guards below cannot fire on an instantiated block: the frontend
/// only emits `vfp.get/set` when the block's binding carries an FPU, and
/// `CortexMBrowserBlock::run` installs the pointer before every call. Stay
/// total in release anyway — a host import has no side-exit channel, so
/// reading through null or past S31 would trade a refusal for a wild read.
fn thumb_vfp_get(host: &ThumbHost, sn: i32) -> i32 {
    let i = sn as u32 as usize;
    if host.fpu.is_null() || i >= 32 {
        return 0;
    }
    unsafe { *host.fpu.add(i) as i32 }
}

fn thumb_vfp_set(host: &ThumbHost, sd: i32, bits: i32) {
    let i = sd as u32 as usize;
    if host.fpu.is_null() || i >= 32 {
        return;
    }
    unsafe {
        *host.fpu.add(i) = bits as u32;
    }
}

/// `vfp.binop(op, a_bits, b_bits)`: the browser compiled lane's arithmetic.
/// Delegates to the same `cortex_m::vfp_binop` the interpreter uses, so the
/// browser and native JITs, and the interpreter, all agree bit-for-bit
/// including FPSCR.FZ/DN and NaN payloads.
fn thumb_vfp_binop(host: &ThumbHost, fop: i32, a: i32, b: i32) -> i32 {
    match VfpBinOp::from_code(fop) {
        Some(op) => vfp_binop(op, a as u32, b as u32, host.fpscr) as i32,
        None => 0,
    }
}

/// Run one compiled Thumb block at `cpu.pc`.
///
/// Returns the number of guest instructions the block committed before it
/// finished or faulted; `0` means the block committed nothing and the caller
/// should interpret one instruction. A non-zero return from a block that
/// stopped on a fault is still charged — the native `run_jit_loop` consumes
/// `actual_n` before it interprets, and dropping the pre-fault instructions
/// makes every fault in a compiled block free of charge, a cycle-accounting
/// divergence from the interpreter.
pub(crate) fn try_browser_cortex_m_jit_step(
    cpu: &mut CortexM,
    bus: &mut SystemBus,
    cache: &mut BrowserJitCache,
    max_n: u32,
) -> u32 {
    if cpu.it_state != 0 {
        return 0;
    }
    if cpu.jit_takeable_exception() {
        return 0;
    }
    let pc = cpu.pc & !1;
    if cache.refused_thumb.contains(&pc) {
        return 0;
    }
    if cache.thumb_get_mut(pc).is_none() {
        let code = bus.read_code_slice(pc as u64, THUMB_CODE_WINDOW);
        if code.len() < 2 {
            cache.refused_thumb.insert(pc);
            cache.refusals = cache.refusals.saturating_add(1);
            return 0;
        }
        let mut frontend = CortexMFrontend::new();
        frontend.set_ram_window(bus.ram.base_addr as u32, bus.ram.data.len() as u32);
        let view = CodeView::new(pc as u64, &code);
        let Ok((plan, binding)) = frontend.translate_block_thumb(pc as u64, &view) else {
            cache.refused_thumb.insert(pc);
            cache.refusals = cache.refusals.saturating_add(1);
            return 0;
        };
        if plan.code.is_empty() || plan.instr_count < THUMB_MIN_PROFITABLE {
            cache.refused_thumb.insert(pc);
            cache.refusals = cache.refusals.saturating_add(1);
            return 0;
        }
        let has_store = binding.map(|b| b.has_store).unwrap_or(false);
        match CortexMBrowserBlock::compile(
            &plan.code,
            binding.is_some(),
            plan.end_pc as u32,
            plan.instr_count,
            has_store,
        ) {
            Ok(block) => {
                cache.compiled_thumb.insert(pc, block);
            }
            Err(e) => {
                web_sys_console_warn(&format!(
                    "labwired-wasm: Cortex-M browser JIT install failed at pc=0x{pc:08x}: {e:?}"
                ));
                cache.refused_thumb.insert(pc);
                cache.refusals = cache.refusals.saturating_add(1);
                return 0;
            }
        }
    }
    let ran = {
        let block = match cache.thumb_get_mut(pc) {
            Some(b) => b,
            None => return 0,
        };
        if block.instr_count == 0 || block.instr_count > max_n {
            return 0;
        }
        if cpu.block_would_cross_irq(bus, block.instr_count) {
            return 0;
        }
        let instr_count = block.instr_count;
        let end_pc = block.end_pc;
        block.run(cpu, &mut bus.ram.data).map(
            |(wire, next_pc, clear_exclusive, fault_pc, fault_retired, fault_it_state)| {
                (
                    wire,
                    next_pc,
                    clear_exclusive,
                    fault_pc,
                    fault_retired,
                    fault_it_state,
                    instr_count,
                    end_pc,
                )
            },
        )
    };
    match ran {
        Ok((
            wire,
            next_pc,
            clear_exclusive,
            fault_pc,
            fault_retired,
            fault_it_state,
            instr_count,
            end_pc,
        )) => {
            if clear_exclusive {
                cpu.clear_exclusive_monitor();
            }
            let (n, cont, needs_interp) = match wire {
                WIRE_FALL_THROUGH => (instr_count, end_pc, false),
                WIRE_CHAIN_DYNAMIC => (instr_count, next_pc, false),
                WIRE_MEM_FAULT | WIRE_UNSUPPORTED => {
                    // Mid-IT resume: the interpreter must see the IT state as
                    // of before the faulting instruction, not a cleared one.
                    cpu.it_state = fault_it_state;
                    (fault_retired, fault_pc, true)
                }
                _ => (instr_count, end_pc, true),
            };
            cpu.pc = cont;
            if needs_interp {
                cache.refusals = cache.refusals.saturating_add(1);
                // The instructions before the fault are still retired and
                // must be charged; only the instruction now at `cpu.pc` needs
                // the interpreter, so return them and let the caller decide.
                return n;
            }
            if n == 0 {
                return 0;
            }
            cache.bump_hit();
            n
        }
        Err(e) => {
            web_sys_console_warn(&format!(
                "labwired-wasm: Cortex-M browser JIT run failed at pc=0x{pc:08x}: {e:?}"
            ));
            cache.refusals = cache.refusals.saturating_add(1);
            0
        }
    }
}

/// Runs up to `max_n` guest instructions of the current window through the
/// browser cache, with the same gates the in-tree `run_jit_loop` applies
/// before every compiled block: a takeable exception ends the window (or is
/// dispatched by the interpreter at zero progress), leftover IT — and every
/// miss — interprets one instruction, and a latching SCB reset or a
/// semihosting `SYS_EXIT` ends the window on the instruction that latched
/// it. The exit check does not take the code; `Machine::advance` does.
///
/// This only decides compiled-vs-interpreted per instruction. The machine
/// boundary around the window (tick cadence, scheduler drains, resets, idle
/// fast forward, work accounting) is core's
/// `Machine::advance_with_window_runner`, so a compiled window and an
/// interpreted window are the same machine cycles.
///
/// Every retirement also advances `bus.current_cycle` in place, because a
/// window is not a machine boundary: models that sync lazily off that
/// accumulator mid-window — SysTick, nRF timers, DWT — read it for every
/// instruction, and the interpreter's `step_batch` (issue #842) and the
/// in-tree `run_jit_loop` both bump it after each retirement. Without the
/// bump a compiled window freezes every one of them for the window's whole
/// duration, which is exactly the JIT-vs-interpreter divergence the
/// browser-layer gate caught on the L476 six-step firmware.
pub(crate) fn run_browser_cortex_m_jit_window(
    cpu: &mut CortexM,
    bus: &mut SystemBus,
    observers: &[Arc<dyn SimulationObserver>],
    config: &SimulationConfig,
    cache: &mut BrowserJitCache,
    max_n: u32,
) -> SimResult<u32> {
    // Same shape as `CortexM::run_jit_loop`: 0 at interval 1, where a window
    // is one instruction and the boundary commit already refreshes the
    // accumulator. Computed outside the loop; the bump stays a no-op write.
    let live_step = u64::from(config.peripheral_tick_interval.max(1) > 1);

    let mut retired = 0u32;
    while retired < max_n {
        let n;
        if cpu.jit_takeable_exception() {
            // Match interpreter `step_batch`: at zero progress the exception
            // is dispatched by `step`; after progress the window ends so the
            // next boundary can take it without a compiled block jumping the
            // dispatch point.
            if retired > 0 {
                break;
            }
            cpu.step(bus, observers, config)?;
            n = 1;
        } else if cpu.it_state != 0 || cpu.waiting_for_event() {
            cpu.step(bus, observers, config)?;
            n = 1;
        } else {
            let block_n = try_browser_cortex_m_jit_step(cpu, bus, cache, max_n - retired);
            n = if block_n > 0 {
                // Compiled instructions skip the interpreter's per-step
                // SysTick consume, so hand the block's cycles to it directly
                // (mirrors `run_jit_loop`). A partial block that stopped on a
                // fault is charged its pre-fault instructions here too; the
                // faulting instruction is left at `cpu.pc` for the next
                // iteration, exactly as `run_jit_loop` leaves it.
                bus.systick_consume_cycles(u64::from(block_n));
                block_n
            } else {
                cpu.step(bus, observers, config)?;
                1
            };
        };
        if live_step != 0 {
            bus.current_cycle += live_step * u64::from(n);
        }
        retired += n;
        if cpu.sysreset_latched()
            || cpu.firmware_exit_latched()
            || (config.idle_fast_forward_enabled && cpu.idle_fast_forward_budget(bus).is_some())
        {
            break;
        }
    }
    Ok(retired)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod firmware_exit_window_tests {
    use super::*;

    /// `movs r2, #1`. Retires only if the window continues after `SYS_EXIT`.
    const MOVS_R2_1: u16 = 0x2201;

    #[test]
    fn browser_window_stops_after_sys_exit_without_taking_the_code() {
        let mut cpu = CortexM::new();
        let mut bus = SystemBus::new();
        bus.write_u16(0, 0xBEAB).unwrap();
        bus.write_u16(2, MOVS_R2_1).unwrap();
        cpu.pc = 0;
        cpu.r0 = 0x18;
        cpu.r1 = 0x20026;
        cpu.r2 = 0;
        let mut cache = BrowserJitCache::new();

        let retired = run_browser_cortex_m_jit_window(
            &mut cpu,
            &mut bus,
            &[],
            &SimulationConfig::default(),
            &mut cache,
            8,
        )
        .unwrap();

        assert_eq!(retired, 1, "the window must stop on SYS_EXIT");
        assert_eq!(cpu.r2, 0, "the instruction after the trap must not retire");
        assert_eq!(cpu.pc, 2);
        assert!(
            cpu.firmware_exit_latched(),
            "the window must not consume the latch"
        );
        assert_eq!(cpu.take_firmware_exit(), Some(0));
        assert!(!cpu.firmware_exit_latched());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod vfp_host_tests {
    use super::*;

    fn with_fpu(fpu: *mut u32) -> ThumbHost {
        ThumbHost {
            ram: std::ptr::null_mut(),
            ram_len: 0,
            fpu,
            fpscr: 0,
        }
    }

    #[test]
    fn vfp_host_round_trips_s_register_bits() {
        let mut fpu = [0u32; 32];
        let host = with_fpu(fpu.as_mut_ptr());

        thumb_vfp_set(&host, 5, 0x3F80_0000);
        assert_eq!(fpu[5], 0x3F80_0000);
        assert_eq!(thumb_vfp_get(&host, 5) as u32, 0x3F80_0000);
    }

    #[test]
    fn vfp_host_refuses_to_touch_a_missing_or_out_of_range_file() {
        let host = with_fpu(std::ptr::null_mut());
        assert_eq!(thumb_vfp_get(&host, 5), 0, "no file, no bits");
        thumb_vfp_set(&host, 5, 0x0BAD_F00D);

        let mut fpu = [0u32; 32];
        let host = with_fpu(fpu.as_mut_ptr());
        assert_eq!(
            thumb_vfp_get(&host, 32),
            0,
            "S32 is outside the 32-register file a Thumb block can name"
        );
        thumb_vfp_set(&host, 32, 0x0BAD_F00D);
        assert!(
            fpu.iter().all(|&w| w == 0),
            "an out-of-range write must not land anywhere in the file"
        );
    }

    #[test]
    fn vfp_host_binop_honors_fpscr_modes() {
        let mut fpu = [0u32; 32];
        let mut host = with_fpu(fpu.as_mut_ptr());

        // FZ: denormal inputs flush to +0 before the add. 0x0000_0001 is
        // 2^-149 — the smallest denormal; 0x1F80_0000 (2^-64) is NORMAL, so
        // it must survive FZ (the control below proves the flush, not the op).
        host.fpscr = 0;
        assert_eq!(
            thumb_vfp_binop(&host, 0, 0x0000_0001, 0x0000_0001) as u32,
            0x0000_0002,
            "without FZ a denormal sum stays denormal"
        );
        host.fpscr = 1 << 24;
        assert_eq!(
            thumb_vfp_binop(&host, 0, 0x0000_0001, 0x0000_0001) as u32,
            0,
            "FZ must flush both denormal inputs before the add"
        );
        assert_eq!(
            thumb_vfp_binop(&host, 0, 0x1F80_0000, 0x1F80_0000) as u32,
            0x2000_0000,
            "FZ must not touch a normal operand"
        );
        // DN: any NaN result becomes the ARM default NaN.
        host.fpscr = 1 << 25;
        assert_eq!(
            thumb_vfp_binop(&host, 0, 0x7FC0_AAAA, 1) as u32,
            0x7FC0_0000
        );
        // Default: first NaN operand propagates quieted.
        host.fpscr = 0;
        assert_eq!(
            thumb_vfp_binop(&host, 0, 0x7F80_0001, 1) as u32,
            0x7FC0_0001
        );
    }
}
