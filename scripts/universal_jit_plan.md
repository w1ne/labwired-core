# Universal browser JIT — design note

Follow-up to PR #131 (Phase 4 prototype). The current prototype hand-crafts WAT
for ONE specific basic block at PC `0x400829cc`. Re-profiling ereader's
steady-state confirmed: the actual hot block (~42% of work, 94k hits per 2M
cycles) is the `loopTask` polling loop at PC `0x400d4a8d`, which contains
L32R / L8UI / BEQZ / **CALL8** / L32R / BEQZ / **J**. The current emitter
covers ADD/SUB/AND/OR/XOR/ADDI/MOVI/EXTUI/L8UI/L32R/MEMW/NOP — so it can NOT
emit this block.

Any user-loaded firmware will have similar control-flow-heavy hot blocks.
A hand-crafted single-block prototype cannot deliver a measurable speedup on
real workloads; we need a runtime-emit walker.

## What's portable from `crates/core/src/cpu/xtensa_jit/bb_multi.rs`

The Phase 3.6.3 native JIT walker (PR #130) ALREADY does runtime emit. It:
- Decodes Xtensa instructions forward from a given PC
- Builds a `Vec<u8>` of wasm module bytes
- Handles 12 opcode families + side-exit on unsupported opcodes
- Caches compiled blocks by PC + PS bits
- Has a hot-counter promotion threshold

**~80% of this code is runtime-agnostic.** It only depends on wasmtime for the
final compile + execute step. The emit logic, opcode coverage, and dispatch
heuristic all carry over.

## What needs replacement for browser

- `wasmtime::Engine` + `Module::new(engine, bytes)` →
  `js_sys::WebAssembly::Module::new(bytes)`
- `wasmtime::Instance::new(...)` →
  `js_sys::WebAssembly::Instance::new(module, imports)`
- Host imports (memory load/store callbacks) currently use wasmtime's
  `Linker::func_wrap`. Browser side needs `wasm-bindgen` exports the JS-side
  imports object can reference.
- Function dispatch goes from `func.call(&mut store, args, &mut results)`
  to `js_sys::Function::apply(this, args)`.

## Opcodes still missing (needed for `loopTask` block + most real hot loops)

- **BEQZ / BNEZ / BEQZ.N / BNEZ.N** — conditional branches. Side-exit pattern:
  emit a `(if ...)` block that returns side-exit-code 1 (taken) or falls
  through.
- **J / J.L** — unconditional jumps. Side-exit code 1 with new PC.
- **CALL8** — windowed call. We already have correctness semantics in
  `xtensa_jit/windowed_call.rs` (Phase 3.6.2). Port to browser emit.
- **RETW.N** — windowed return. Side-exit code 1 with new PC = return addr.
- **MOV.N / ADD.N / ADDI.N / MOVI.N** — narrow forms (already supported in
  bb_multi.rs's emit; just confirm they're exposed in the browser emit too).

## Estimated complexity

| Phase | Scope | LOC | Time |
|---|---|---|---|
| 4.1 | Extract bb_multi emit into a runtime-agnostic core | ~300 | 1 week |
| 4.2 | Browser-side runtime (Module::new, Instance::new, host imports) | ~400 | 1 week |
| 4.3 | BEQZ/BNEZ/J emit + lockstep | ~200 | 3 days |
| 4.4 | CALL8/RETW emit + lockstep | ~300 | 1 week |
| 4.5 | Multi-block cache + invalidation in browser | ~200 | 3 days |
| 4.6 | Bench + tune hot-counter threshold | ~50 | 1 day |
| **Total** | | **~1450** | **~4 weeks** |

## Honest ceiling

Even with universal browser JIT, the expected speedup on ereader is limited by:
1. Cross-language host-import cost (memory access goes through JS callbacks)
2. Browser wasm engines have less aggressive optimization than wasmtime native
3. The hot block in ereader is small enough that wasm compile overhead may
   dominate

Best estimate: 1.5-2× browser speedup, not 5-10×. The real-time goal (matching
240MHz silicon) remains out of reach without AOT-from-flash translation.

## Recommendation

Don't ship Phase 4.0 (the hand-crafted prototype). It works as a feasibility
study but produces no measurable user-visible speedup, and hand-crafting WAT
per firmware doesn't scale. Either:

1. **Commit to Phases 4.1-4.6** (~4 weeks) and ship the universal browser JIT
   once it shows ≥1.5× on ereader cold boot
2. **Pause Phase 4 entirely** and focus on snapshot-based fast-boot (labwired-
   core#122 UC8151D last-displayed framebuffer) which gives ~1s "paint" for
   ZERO compute work — the right answer for a demo
