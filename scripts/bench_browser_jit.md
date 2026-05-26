# Browser-side JIT benchmark protocol (#124 Phase 4)

This document captures the exact protocol used to measure the browser-side
JIT prototype against the pure-interpreter baseline.  The prototype lives
in `crates/wasm/src/jit_browser.rs`; the JS entry points are
`set_jit_enabled(bool)`, `bench_jit(cycles) -> millis`, `jit_hits()`, and
`jit_refusals()` on `WasmSimulator`.

## Quick run (playground devtools)

1. Open <https://app.labwired.com/playground> with the labwired-ereader
   firmware loaded.
2. Get a handle to the underlying `WasmSimulator` (the playground exposes
   it on `window.__lw_sim` in dev builds; if not, run a build with that
   shim or attach via the `simulator` field of the React store).
3. In devtools console:

   ```js
   const sim = window.__lw_sim;
   // Baseline — JIT disabled.
   sim.set_jit_enabled(false);
   const t_baseline = sim.bench_jit(10_000_000);
   const r_baseline_cycs_per_sec = 10_000_000 / (t_baseline / 1000);
   console.log(`baseline: ${t_baseline.toFixed(0)} ms (${r_baseline_cycs_per_sec.toFixed(0)} cyc/s)`);

   // With JIT.
   sim.set_jit_enabled(true);
   const t_jit = sim.bench_jit(10_000_000);
   const r_jit_cycs_per_sec = 10_000_000 / (t_jit / 1000);
   console.log(`with JIT: ${t_jit.toFixed(0)} ms (${r_jit_cycs_per_sec.toFixed(0)} cyc/s, hits=${sim.jit_hits()}, refusals=${sim.jit_refusals()})`);

   console.log(`speedup: ${((t_baseline - t_jit) / t_baseline * 100).toFixed(1)}%`);
   ```

## What we expect

Per #124 Phase 3.6.3 native-JIT measurement: the 0x400829cc hot block is
~82% of all ereader work.  Replacing the interpreter loop for that block
with a single wasm call should compound across ~900k hits per 10M-cycle
ereader run.

The native JIT delivered ~+13% in #130.  In the browser, per-instruction
interpreter cost is **higher** (no native-compiled Rust; everything goes
through wasm bytecode in the host engine), so the per-block savings should
be larger in absolute terms.  Realistic outcomes:

* +10–15% — matches the native-side speedup.  Best case.
* +0–10% — JS<->wasm call overhead eats a chunk of the win; still a
  positive result and validates the architecture.
* < 0%   — wasm-from-wasm crossing is too expensive in browser engines.
  Negative result; informs whether to pursue Phase 4.1 (multi-block JIT)
  or pivot to a different strategy (interpreter-level micro-ops, etc.).

## Sanity checks before trusting numbers

* `sim.jit_hits()` must be > 0 after the JIT-enabled run.  If 0, the PC
  never reached `0x400829cc` and the bench measured nothing about the
  JIT — re-run with a longer cycle budget or verify the firmware was
  built with the same Arduino-ESP32 toolchain that produced the BB
  profile.
* `sim.jit_refusals()` should be 0 (or very low — single-digit) for an
  ereader workload.  A high refusal count means the host import path
  is rejecting the wasm call; check console.warn output.
* Run each variant 3+ times and take the median.  First-run numbers
  include the V8 compile cost for the JITed module (~few ms).
