# JIT trace fusion v2 — porting m1 onto the universal-dispatch framework

Status: design note only (no code yet). Ports the July `perf/jit-superblock-m1`
mechanism onto the current `crates/core/src/cpu/jit_framework/` and extends it
to the Cortex-M frontend, which now needs it more than RISC-V does (2.4–4.8x
slower than the batched interpreter on real firmware; browser Thumb path never
compiles blocks of 1–3 instructions).

## What m1 did (July, pre-framework `cpu/riscv.rs` + early `jit_framework/riscv`)

- Statically walks forward from a block's entry, following only *resolvable*
  successors (fallthrough / direct branch targets), building a trace of N
  basic blocks.
- Registers live across interior block boundaries stay in wasm locals for the
  whole trace; only one entry-load and one exit-store of the guest register
  file happens, instead of once per block. Liveness is unioned over the trace.
- Interior control flow becomes an if-chain dispatch over a `next_pc` local
  inside one wasm function, instead of N separate host<->guest transitions.
- Correctness-critical: a per-trace *retired-instruction budget* computed at
  trace entry — `min(max_count - retired, mtimecmp - mtime - 1 if timer armed)`
  — checked before each interior block (`retired + n_k > budget` -> exit to
  host at that block's PC). This is what lets a fused trace still land exactly
  on a tick/IRQ boundary, mirroring the existing per-block guard.
- Any block containing a dynamic terminator (JALR/C.JR), MMIO access,
  unmodeled instruction, or an out-of-window load/store is never fused past —
  it is a trace-ending block, and the trace exits to the host there as today.
- Gated behind `LW_JIT_TRACE`, default off. Byte-identical when off.

Measured (C3 OLED, warm, 30M instrs): host calls 8.44M -> 4.75M, block_instrs
3.14 -> 5.69, warm MIPS 47.1 -> 59.3 (+26%), coverage 89.7% -> 91.5%.

## Where it lands in the current framework

`crates/core/src/cpu/jit_framework/` now has `riscv/` and `cortex_m/` frontends
sharing `dispatch.rs`, `block_cache.rs`, `runtime.rs`, `side_exit.rs`,
`frontend.rs`, `differential.rs`. Both frontends compile one basic block to one
wasm function today (`emit.rs::emit_block`, roughly), register them in
`block_cache.rs`, and `run_jit_loop` (in `runtime.rs`/`dispatch.rs`) chains
Ready->Ready block executions with a host-side dispatch loop between them —
exactly the per-block host<->guest transition m1 eliminated for the interior
of a trace.

Diff against `perf/jit-superblock-m1` shows heavy drift since July in the
touched files (`riscv/exec.rs` +595/-203 lines, `cpu/riscv.rs` +1011/-165,
`riscv/emit.rs` +456/-22) — this is not a clean cherry-pick. The mechanism
must be re-derived against current types:

- **Trace walker**: new code in `block_cache.rs` or a new
  `jit_framework/trace.rs` shared by both frontends. Walks forward from a
  block's decoded terminator using the frontend's `Frontend` trait (in
  `frontend.rs`) to ask "is this block's successor statically resolvable and
  not already a trace-ending condition" — same criteria as m1: direct
  branch/fallthrough only, stop at first dynamic terminator / MMIO access /
  unmodeled op / cold target.
- **Interior-boundary budget check**: computed once from the framework's
  existing deadline contract — `advance_with_window_runner` (#1128) already
  hands blocks a step/tick window; the fused-trace entry must compute the same
  `min(remaining_steps, ticks_until_next_deadline)` the per-block path already
  derives, and each interior block emits the same `retired + n_k > budget`
  check the current per-block code does via `jit_takeable_exception` /
  `block_would_cross_irq` — just moved to interior trace positions rather than
  only at the top of `run_jit_loop`.
- **Registers in locals across boundaries**: `emit.rs` per-ISA register
  load/store sequences currently bracket a single block; for a trace they move
  to bracket the whole fused wasm function, with liveness unioned over all
  member blocks (same technique m1 used, ported to whatever register model
  `riscv/emit.rs` and the new `cortex_m/emit.rs` use today — the two ISAs will
  need their own liveness/union logic since register files differ).
- **Side exits**: every trace-ending condition (dynamic terminator, budget
  exceeded, MMIO, unmodeled, cold target, fault) reuses `side_exit.rs`'s
  existing stub-generation, parameterized by which block-interior PC the exit
  fires from, instead of only the block's own end.
- **EngineStats**: extend with per-trace counters (trace length, host-call
  reduction, budget-exit count) analogous to the July coverage bench, surfaced
  through the same `EngineStats` struct `jit-stats` output already reports.

## Correctness invariants (must hold for both ISAs)

1. A fused trace must never retire an instruction past the point where the
   per-instruction interpreter would have stopped: batch/step budget, armed
   timer/IRQ deadline (`jit_takeable_exception`, `block_would_cross_irq`), or
   the end of the current `advance_with_window_runner` window.
2. Any MMIO access, unmodeled instruction, or dynamic control-flow terminator
   ends the trace at that instruction — never fused past.
3. The interpreter remains the spec of record: fusion is a pure performance
   transform on statically-known control flow; it must produce byte-identical
   guest state to the interpreter on every differential/lockstep gate, on
   every ISA, with the option on or off.
4. Off by default until lockstep suites pass with it on; toggled via an engine
   option (`LABWIRED_RISCV_JIT_TRACE` / `LABWIRED_CORTEX_M_JIT_TRACE` or an
   `EngineOptions` field), independent per frontend so RISC-V and Cortex-M can
   be enabled/rolled back independently.

## Remaining work (not done in this pass)

Implementation in `jit_framework` (trace walker, per-ISA emit changes, budget
checks, side-exit wiring), enabling for Cortex-M, the full lockstep/clippy/fmt
gate pass, and the three-fixture wall-clock benchmark are substantial
follow-on engineering — estimated multi-day, not completed in this session.
