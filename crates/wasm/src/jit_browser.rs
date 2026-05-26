// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Browser-side Xtensa JIT prototype (#124 Phase 4).
//!
//! ## Why this exists
//!
//! The native JIT in `labwired-core::cpu::xtensa_jit` runs hot Xtensa
//! basic blocks as WebAssembly modules via wasmtime. Wasmtime doesn't
//! compile for `wasm32-unknown-unknown`, so the deployed browser sim
//! has been paying full interpreter cost for every instruction —
//! including the dominant 0x400829cc hot block (~82% of ereader work).
//!
//! This module is the browser counterpart: it takes the **same wasm
//! bytes** the native JIT consumes (baked at compile time by
//! `crates/core/build.rs`), instantiates them through
//! `js_sys::WebAssembly::{Module, Instance}`, and exposes a Rust API
//! that calls the resulting module on the hot path.
//!
//! ## Decoupling
//!
//! Emit is fully decoupled from runtime:
//!   1. `crates/core/src/cpu/xtensa_jit/hot_bb.wat` — single source of
//!      truth for the JIT body.
//!   2. `crates/core/build.rs` — compiles WAT → wasm bytes at crate
//!      build time. Build-deps only; doesn't leak into the runtime
//!      dependency tree.
//!   3. `labwired_core::cpu::xtensa_jit_bytes::HOT_BB_WASM` — bytes
//!      accessible from any crate, with or without `--features jit`.
//!   4. Native (wasmtime) backend uses `Module::new(engine, HOT_BB_WASM)`.
//!   5. Browser (this module) uses
//!      `js_sys::WebAssembly::Module::new(Uint8Array(HOT_BB_WASM))`.
//!
//! ## Host import surface
//!
//! The JITed block imports `host.read_u8(addr: i32) -> i32`. The host
//! returns `-1` to signal a bus error; the wasm body then returns
//! exit code `5` (EXIT_HOST_BUS_ERROR) and the caller falls back to
//! the interpreter. To match the native backend's "pre-staged byte
//! queue" model, the browser path stages a small Vec on the host side
//! and dequeues from a JS closure each time the import is invoked.
//!
//! ## Scope cap (#124 Phase 4)
//!
//! * One block JIT'd (0x400829cc — the dominant 82%-of-work hot BB).
//! * No BB walker, no cache management, no peripheral dispatch from
//!   inside wasm. This is a prototype to measure whether the
//!   browser-wasm-inside-browser-wasm architecture pays off; if it
//!   does, generalising is a Phase 4.1+ follow-up.

use js_sys::{Array, Function, Object, Reflect, Uint8Array, WebAssembly};
use labwired_core::cpu::xtensa_jit_bytes::{
    EXIT_FALL_THROUGH, EXIT_HOST_BUS_ERROR, HOT_BB_END, HOT_BB_INSTR_COUNT, HOT_BB_L32R_ADDR,
    HOT_BB_PC, HOT_BB_WASM,
};
use labwired_core::cpu::xtensa_sr::CCOUNT;
use labwired_core::cpu::XtensaLx7;
use labwired_core::Bus;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Result of running the hot-block wasm body in the browser.
pub struct BrowserMultiOpResult {
    pub exit_code: i32,
    pub a2: u32,
    pub a6: u32,
    pub a8: u32,
    pub a10: u32,
}

/// Browser-side handle for the JITed hot block. Holds the compiled
/// `WebAssembly.Module`, the instantiated `Instance`, a long-lived JS
/// closure for the `host.read_u8` import, and the shared host-side
/// byte queue.
///
/// Lifetimes: the closure is reachable from JS as long as this struct is
/// alive (the JS-side import refers back into it). Drop order matters:
/// we drop the `_keepalive` field last so the JS engine can never call
/// a stale closure. Rust drops fields top-to-bottom; we order them so
/// that the closure outlives the instance.
pub struct BrowserHotBbJit {
    /// Cached `run` export. Calling this dispatches into wasm.
    run: Function,
    /// Host-side queue of pre-staged byte values, dequeued by the JS
    /// shim each time wasm invokes `host.read_u8`. RefCell + Rc because
    /// (a) the closure needs a long-lived clone and (b) the queue is
    /// mutated both from `stage_loads` and from inside the closure.
    pending: Rc<RefCell<Vec<u8>>>,
    /// Number of times `run` has been invoked. Surfaced for the
    /// `bench_jit` benchmark.
    pub hits: u64,
    /// Keep the closure alive for the lifetime of the instance.
    /// Dropping this severs the JS-side import — must outlive any
    /// possible call into `run`.
    _read_u8_closure: Closure<dyn FnMut(i32) -> i32>,
    /// `Instance` is kept alive to root the imports' lifetimes. Held
    /// after `run` (which references it through JS) so drop order is
    /// safe.
    _instance: WebAssembly::Instance,
}

impl BrowserHotBbJit {
    /// Compile + instantiate the hot-block module. Failure modes:
    ///   * `WebAssembly.Module(buffer)` rejects the bytes — extremely
    ///     unlikely given the bytes pass wasmtime validation already.
    ///   * `WebAssembly.Instance(module, imports)` errors — usually a
    ///     mismatch in the import object shape; the construction here
    ///     is exhaustively typed so this should not happen in practice.
    ///   * `instance.exports.run` is not a Function — shouldn't happen
    ///     for a module that declares an exported function named `run`,
    ///     but we surface it as JsValue rather than panicking.
    pub fn compile() -> Result<Self, JsValue> {
        // 1. Wrap the static wasm bytes in a `Uint8Array` view. We
        //    `copy_from` to avoid handing the engine a backing buffer
        //    that we still hold (the Rust slice is `'static` so it's
        //    not unsafe to share, but copy is cheap and removes a
        //    subtle lifetime concern).
        let buf = Uint8Array::new_with_length(HOT_BB_WASM.len() as u32);
        buf.copy_from(HOT_BB_WASM);

        // 2. Compile. `Module::new` accepts `&JsValue` so we pass the
        //    typed-array up-casted.
        let module = WebAssembly::Module::new(&buf.into())
            .map_err(|e| JsValue::from_str(&format!("WebAssembly.Module: {e:?}")))?;

        // 3. Build the host-side byte queue and the JS closure that
        //    dequeues from it. The closure captures an `Rc<RefCell<…>>`
        //    so the queue is reachable from both Rust (`stage_loads`)
        //    and JS (each invocation of `host.read_u8`).
        let pending: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::with_capacity(2)));
        let pending_for_closure = pending.clone();
        let read_u8_closure = Closure::<dyn FnMut(i32) -> i32>::new(move |_addr: i32| -> i32 {
            // Wasm passes the address but we don't need it — the host
            // pre-staged the bytes in BB order. If the queue is empty
            // we report a bus error; the wasm body returns exit=5.
            let mut q = pending_for_closure.borrow_mut();
            if q.is_empty() {
                return -1;
            }
            q.remove(0) as i32
        });

        // 4. Build the imports object: { host: { read_u8: closure } }.
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

        // 5. Instantiate.
        let instance = WebAssembly::Instance::new(&module, &imports)
            .map_err(|e| JsValue::from_str(&format!("WebAssembly.Instance: {e:?}")))?;

        // 6. Pluck `run` out of the exports. `exports` is a frozen JS
        //    object; `Reflect::get` returns the function value.
        let exports = instance.exports();
        let run_val = Reflect::get(&exports, &JsValue::from_str("run"))
            .map_err(|e| JsValue::from_str(&format!("get exports.run: {e:?}")))?;
        let run: Function = run_val
            .dyn_into::<Function>()
            .map_err(|_| JsValue::from_str("exports.run is not a Function"))?;

        Ok(Self {
            run,
            pending,
            hits: 0,
            _read_u8_closure: read_u8_closure,
            _instance: instance,
        })
    }

    /// Stage the byte values the wasm body's L8UI ops will receive.
    /// `bytes.len()` must equal the BB's L8UI count (2 for the hot BB).
    pub fn stage_loads(&self, bytes: &[u8]) {
        let mut q = self.pending.borrow_mut();
        q.clear();
        q.extend_from_slice(bytes);
    }

    /// Invoke the hot block. Returns the 5-tuple `(exit, a2, a6, a8, a10)`
    /// produced by the wasm body, decoded back to Rust types.
    ///
    /// JS-side, `run(...)` returns a JS Array because wasm multi-value
    /// returns map to arrays in the WebAssembly JavaScript API. We
    /// pluck five `i32`s back out via `Reflect::get` + `as_f64()` —
    /// JS numbers cleanly hold any i32 round-trip.
    pub fn run(&mut self, a3: u32, a5: u32, l32r_val: u32) -> Result<BrowserMultiOpResult, JsValue> {
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
        let g = |i: u32| -> i32 {
            arr.get(i)
                .as_f64()
                .map(|f| f as i64 as i32)
                .unwrap_or(0)
        };
        self.hits += 1;
        Ok(BrowserMultiOpResult {
            exit_code: g(0),
            a2: g(1) as u32,
            a6: g(2) as u32,
            a8: g(3) as u32,
            a10: g(4) as u32,
        })
    }
}

/// Lazily-built process-wide JIT cache for the browser. Mirrors the
/// `JitCache` shape on the native side but holds only the hot BB —
/// scope cap for the prototype (#124 Phase 4).
///
/// We keep this thread-local because:
///   * The browser sim is single-threaded (wasm32 main thread).
///   * `js_sys` types are not `Send`, ruling out global statics.
pub struct BrowserJitCache {
    pub hot_bb: Option<BrowserHotBbJit>,
    pub refusals: u64,
}

impl Default for BrowserJitCache {
    fn default() -> Self {
        Self {
            hot_bb: None,
            refusals: 0,
        }
    }
}

impl BrowserJitCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lazily compile + return the hot block. Returns `None` only if
    /// JS-side compilation/instantiation failed; the caller logs and
    /// proceeds with the interpreter.
    pub fn lookup_or_install(&mut self) -> Option<&mut BrowserHotBbJit> {
        if self.hot_bb.is_none() {
            match BrowserHotBbJit::compile() {
                Ok(b) => self.hot_bb = Some(b),
                Err(e) => {
                    web_sys_console_warn(&format!(
                        "labwired-wasm: browser JIT compile failed: {e:?}. Falling back to interpreter."
                    ));
                    return None;
                }
            }
        }
        self.hot_bb.as_mut()
    }
}

/// Attempt to dispatch the current PC into the browser JIT. Returns
/// `true` if the JIT handled the step (caller does NOT call
/// `Cpu::step` for this iteration); `false` otherwise.
///
/// The semantics mirror `XtensaLx7::try_jit_multi_op` in the native
/// path:
///   * Pre-read both L8UI bytes through the live bus. If either bus
///     read errors, refuse the JIT.
///   * Pre-resolve the L32R literal.
///   * Stage the bytes, invoke wasm.
///   * Commit registers + advance PC + bump CCOUNT iff exit=0.
///
/// We don't run the IRQ / pre-fetch path here — those happen on the
/// regular interpreter step. The JIT is a fast-path for the hot BB
/// only; the caller's loop falls back to the normal step path for
/// every other PC.
pub fn try_browser_jit_step(
    cpu: &mut XtensaLx7,
    bus: &mut dyn Bus,
    cache: &mut BrowserJitCache,
) -> bool {
    if cpu.pc != HOT_BB_PC {
        return false;
    }
    // Pre-read both L8UI bytes through the live bus.
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
    let l32r_val = match bus.read_u32(HOT_BB_L32R_ADDR as u64) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let block = match cache.lookup_or_install() {
        Some(b) => b,
        None => return false,
    };
    block.stage_loads(&[b0, b1]);
    let res = match block.run(a3, a5, l32r_val) {
        Ok(r) => r,
        Err(_) => {
            cache.refusals += 1;
            return false;
        }
    };
    match res.exit_code {
        x if x == EXIT_FALL_THROUGH => {
            cpu.regs.write_logical(10, res.a10);
            cpu.regs.write_logical(6, res.a6);
            cpu.regs.write_logical(2, res.a2);
            cpu.regs.write_logical(8, res.a8);
            cpu.pc = HOT_BB_END;
            // CCOUNT honesty: the interpreter would have advanced
            // CCOUNT by HOT_BB_INSTR_COUNT (one per instruction). We
            // mirror that here so the timer interrupt + CCOMPARE0
            // edge logic still fires correctly.
            let cc = cpu.sr.read(CCOUNT);
            cpu.sr.write(CCOUNT, cc.wrapping_add(HOT_BB_INSTR_COUNT));
            cpu.branched = false;
            true
        }
        x if x == EXIT_HOST_BUS_ERROR => {
            cache.refusals += 1;
            false
        }
        _ => {
            cache.refusals += 1;
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
