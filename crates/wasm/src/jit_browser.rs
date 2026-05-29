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
//! ## Host import surface (Phase 4.2 scope)
//!
//! The emit-core today produces a single import: `host.read_u8(i32) ->
//! i32`. Backend behaviour: the host pre-stages the L8UI bytes via
//! [`BrowserCompiledBlock::stage_loads`], the closure dequeues from
//! that shared [`Rc<RefCell<Vec<u8>>>`]. If the queue is empty the
//! closure returns `-1` and the wasm body exits with
//! [`EXIT_HOST_BUS_ERROR`]; the dispatcher treats that as a refusal.
//!
//! Phase 4.3 will add `host.read_u32` / `host.write_u32` /
//! `host.branch_target` imports as variable-length-block emit lands;
//! [`build_imports`] is structured so wiring more imports is
//! additive.
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
//!   4. Pre-resolve any host-input values the block needs (today: two
//!      L8UI bytes + one L32R literal).
//!   5. Call `run(a3, a5, l32r_val)`, marshal the return tuple back
//!      into the CPU register file, advance PC to [`EmittedBlock::end_pc`],
//!      bump CCOUNT by `length_in_instrs - 1` (the outer step already
//!      counted one).

use js_sys::{Array, Function, Object, Reflect, Uint8Array, WebAssembly};
use labwired_core::cpu::xtensa_jit::emit_core::{self, EmitError, EmittedBlock, PsBits};
use labwired_core::cpu::xtensa_jit_bytes::{
    EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, HOT_BB_L32R_ADDR, HOT_BB_PC,
};
use labwired_core::cpu::xtensa_sr::CCOUNT;
use labwired_core::cpu::XtensaLx7;
use labwired_core::Bus;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
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

/// One installed block: the compiled `WebAssembly.Module`, its
/// `Instance`, the cached `run` export, and the host-side load queue
/// the `host.read_u8` import dequeues from.
///
/// Drop order: Rust drops fields top-to-bottom. We list the closure
/// AFTER `run` and `_instance` so the JS-reachable import is still
/// alive whenever `run` could conceivably be called. Once we stop
/// invoking `run` (by dropping the whole struct), the closure can be
/// torn down safely.
pub struct BrowserCompiledBlock {
    /// Exported `run` function — the wasm body of the emitted block.
    /// Cached as a `Function` so dispatch is a direct `call3` with no
    /// `Reflect::get` per invocation.
    run: Function,
    /// Host-side queue of pre-staged byte values. The closure dequeues
    /// from this each time wasm invokes `host.read_u8`. RefCell+Rc
    /// because (a) the closure holds a long-lived clone, (b)
    /// `stage_loads` mutates it from outside the closure.
    pending: Rc<RefCell<Vec<u8>>>,
    /// emit-core's view of the block — kept for `length_in_instrs`,
    /// `end_pc`, and the side-exit reason map. Cheap to clone (a Vec
    /// of bytes + small metadata) and we only do it once at install.
    emitted: EmittedBlock,
    /// Hit counter. Surfaced as `WasmSimulator::jit_hits()` so the
    /// bench harness can confirm the JIT actually fired.
    pub hits: u64,
    /// Closure must outlive the instance: the JS-side imports table
    /// references it. Dropping it while wasm could still call back
    /// would dangle. Leading underscore: never read in Rust, only
    /// holds the closure alive.
    _read_u8_closure: Closure<dyn FnMut(i32) -> i32>,
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

        // 3. Build the host-side queue + the JS closure that dequeues
        //    from it. The closure captures the queue via `Rc` so both
        //    Rust (`stage_loads`) and JS (each `host.read_u8` call) can
        //    reach it.
        let pending: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::with_capacity(4)));
        let pending_for_closure = pending.clone();
        let read_u8_closure = Closure::<dyn FnMut(i32) -> i32>::new(move |_addr: i32| -> i32 {
            // Wasm passes the address but we don't need it — the host
            // pre-staged the bytes in BB order. Empty queue ⇒ bus
            // error; wasm body returns EXIT_HOST_BUS_ERROR.
            let mut q = pending_for_closure.borrow_mut();
            if q.is_empty() {
                return -1;
            }
            q.remove(0) as i32
        });

        // 4. Build imports + instantiate.
        let imports = build_imports(&read_u8_closure)?;
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
            pending,
            emitted,
            hits: 0,
            _read_u8_closure: read_u8_closure,
            _instance: instance,
        })
    }

    /// Stage the byte values the wasm body's L8UI ops will receive.
    /// Caller must supply exactly as many bytes as the block expects;
    /// extras are ignored, shortages surface as `EXIT_HOST_BUS_ERROR`
    /// inside wasm.
    pub fn stage_loads(&self, bytes: &[u8]) {
        let mut q = self.pending.borrow_mut();
        q.clear();
        q.extend_from_slice(bytes);
    }

    /// Invoke the block. Returns the 5-tuple `(exit, a2, a6, a8, a10)`
    /// produced by the wasm body.
    ///
    /// JS-side, multi-value wasm returns become Arrays. We pluck five
    /// `i32`s out via `Array::get` + `as_f64` — JS numbers round-trip
    /// every i32 cleanly.
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
        let arr: Array = result
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

    /// Expose the block's emit-core metadata. Used by the dispatcher
    /// to advance PC and bump CCOUNT after a clean fall-through.
    pub fn emitted(&self) -> &EmittedBlock {
        &self.emitted
    }
}

/// Build the JS `imports` object the wasm module expects. Today the
/// emit-core produces only `host.read_u8`; Phase 4.3 will grow this to
/// `read_u32` / `write_u32` / `branch_target` as variable-length emit
/// lands. Structuring this as a separate helper keeps the additions
/// surgical.
fn build_imports(read_u8_closure: &Closure<dyn FnMut(i32) -> i32>) -> Result<Object, JsValue> {
    let host_obj = Object::new();
    Reflect::set(
        &host_obj,
        &JsValue::from_str("read_u8"),
        read_u8_closure.as_ref().unchecked_ref(),
    )
    .map_err(|e| JsValue::from_str(&format!("set host.read_u8: {e:?}")))?;

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

    // Pre-read host-input values. Today's emit (the canonical hot BB)
    // needs two L8UI bytes from [a3, a3+1] and the L32R literal at
    // HOT_BB_L32R_ADDR. Phase 4.3+ will need a more general staging
    // model — at that point the EmittedBlock will carry a manifest of
    // required inputs.
    let a3 = cpu.regs.read_logical(3);
    let a5 = cpu.regs.read_logical(5);
    let b0 = match bus.read_u8(a3 as u64) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let b1 = match bus.read_u8((a3.wrapping_add(1)) as u64) {
        Ok(v) => v,
        Err(_) => return false,
    };
    // L32R address is currently hardcoded for the hot block. Phase 4.3
    // will move this into EmittedBlock alongside the rest of the input
    // staging manifest.
    let l32r_addr = if pc == HOT_BB_PC { HOT_BB_L32R_ADDR } else { 0 };
    let l32r_val = match bus.read_u32(l32r_addr as u64) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let block = match cache.get_mut(pc, ps_bits) {
        Some(b) => b,
        None => return false,
    };
    let end_pc = block.emitted().end_pc;
    let length_in_instrs = block.emitted().length_in_instrs;

    block.stage_loads(&[b0, b1]);
    let res = match block.run(a3, a5, l32r_val) {
        Ok(r) => r,
        Err(_) => {
            cache.refusals = cache.refusals.saturating_add(1);
            return false;
        }
    };

    match res.exit_code {
        x if x == EXIT_FALL_THROUGH => {
            cpu.regs.write_logical(10, res.a10);
            cpu.regs.write_logical(6, res.a6);
            cpu.regs.write_logical(2, res.a2);
            cpu.regs.write_logical(8, res.a8);
            cpu.pc = end_pc;
            // CCOUNT honesty: the interpreter would have advanced
            // CCOUNT by length_in_instrs - 1 (one per instruction; the
            // outer step counts one more on its own). Mirror the
            // native `try_jit_multi_op` path. If a future emit ever
            // produces a 0-length block, the saturating_sub keeps the
            // arithmetic sane.
            if length_in_instrs > 1 {
                let cc = cpu.sr.read(CCOUNT);
                cpu.sr.write(CCOUNT, cc.wrapping_add(length_in_instrs - 1));
            }
            cpu.branched = false;
            cache.bump_hit();
            true
        }
        x if x == EXIT_HOST_BUS_ERROR => {
            cache.refusals = cache.refusals.saturating_add(1);
            false
        }
        _ => {
            cache.refusals = cache.refusals.saturating_add(1);
            false
        }
    }
}

/// Console warn shim — avoids pulling in `web-sys` just for `console`.
/// `console.warn` is universally available; we go through wasm-bindgen
/// js_namespace = console directly.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn web_sys_console_warn(s: &str);
}
