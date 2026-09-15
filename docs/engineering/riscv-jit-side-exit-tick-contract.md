# RISC-V JIT side-exit and tick contract

**Date:** 2026-09-15  
**Status:** research note (no Thumb emit). Sources are in-tree code plus QEMU TCG / tlib / Simics docs cited at the end.

This is the missing write-up before any Cortex-M **codegen**. #1121 (all-bail Thumb walker + lockstep) is plumbing only. Emitting wasm for Thumb is **not** licensed by that PR.

## 1. What the industry actually does

| System | Translation | RAM | MMIO | Virtual time |
|---|---|---|---|---|
| QEMU TCG | Basic blocks, cached by physical PC | Softmmu TLB **addend**; stay in translated code | TLB MMIO flag → helper / often **end TB** | `icount`: N ns/instruction, **not** cycle-accurate ([tcg-icount.rst](https://github.com/qemu/qemu/blob/master/docs/devel/tcg-icount.rst)) |
| tlib (Renode) | QEMU-derived TBs, LGPL 2.1 | Inline TLB | Bus callback into C#; `pagesAccessedByIo` | `PerformanceInMips` (default 100) → virtual µs ([Renode time framework](https://docs.swedishembedded.com/renode-docs/advanced/time_framework.html)) |
| Simics | JIT / interpret / idle hypersim | Fast path | Device models | Separate **realtime** vs as-fast-as-possible |
| LabWired RISC-V JIT | wasmtime blocks | Copy RAM window in/out; inline load/store **in window** | **Side-exit**, interpreter does the access | **1 cycle / insn** at `cpu_hz`; peripherals tick every `peripheral_tick_interval` |

LabWired is closer to QEMU’s RAM-fast / MMIO-slow split than to Renode’s MIPS clock. It is **not** tlib. Do not import tlib.

## 2. Interpreter remains the spec

From `jit_framework/fallback.rs`:

> The JIT compiles *away* per-instruction structure: a block retires many guest instructions as one host call.

Forced **interpreter** when any of:

- observers non-empty
- breakpoints non-empty
- logic probes armed
- `requires_cycle_accurate()` (HC-SR04, IO-Link, modelled FLASH, …)

Production gate: `RiscV::jit_gate_allows` (`cpu/riscv.rs`).

QEMU analogue: `one-insn-per-tb` / not chaining across MMIO. Same idea: **visibility beats speed**.

## 3. Side-exit vocabulary (ISA-neutral)

`jit_framework/side_exit.rs`:

| Exit | Meaning |
|---|---|
| `Chain { next_pc }` | Block finished; dispatcher may enter another compiled block |
| `EnterInterpreter { resume_pc, reason }` | Interpreter must run **at least one** insn at `resume_pc` |
| `Exception { resume_pc, cause }` | Synchronous fault; machine exception path |

`BailReason::MemoryFault`: load/store effective address **outside** the bound RAM window (MMIO / unmapped). Interpreter **replays that instruction** on the real bus.

This matches QEMU: MMIO is never the TB fast path; device models stay in C (here: Rust `Bus`).

## 4. RISC-V emit contract (what Thumb must copy if it emits)

From `jit_framework/riscv/emit.rs` (chunks C/D/E):

1. **ALU / branches** operate on wasm locals mirroring `x0..x31`.
2. **RAM** is one contiguous window (`bus.ram`). In-window `lw`/`sw` are inline wasm loads/stores.
3. **Out of window** → write faulting PC + retired-before-fault count → `WIRE_MEM_FAULT` → interpreter at **that PC** (re-execute the load/store; prior insn writeback already flushed).
4. Compiled blocks **never call the bus**. So they never see UART, CLINT MMIO, or GPIO. Clock CSRs that are MMIO/mtime are **not** sampled mid-block.
5. After a successful block, `run_jit_loop` does `update_mtime_after_elapsed_cycles(actual_n)` so CLINT `mtime` / MTIP match interpreter `+1` per insn (`cpu/riscv.rs`).

**Thumb analogue (not implemented):** SysTick CURRENT, NVIC pending, DWT CYCCNT if modelled. A block of `n` instructions is `n` cycles. If SysTick would underflow in those `n` cycles, **do not run the block** — same as `block_would_cross_irq`.

## 5. Tick interval and batching (the fidelity rail)

Peripherals tick every `peripheral_tick_interval` cycles (1 = every insn; 64/512 = batched).

`run_jit_loop` mirrors the interpreter batch:

- Never retire past `max_count` (the batch `Machine::run` already clamped to the next event).
- `block_would_cross_irq(n)`: if `mstatus.MIE` and the block’s `mtime` span would cross `mtimecmp` (or IRQ already pending), **interpret one**.
- After each dispatch: if `idle_fast_forward_enabled` and CPU is parked, **return the batch early** (same as interpreter).
- If `peripheral_tick_interval > 1` and `retired >= earliest_pending_deadline - batch_start`, **return**.
- With event-scheduler and tick > 1: `publish_cycle(batch_start + retired)` **before each** dispatch so an **interpreted** MMIO read sees the cycle-exact clock. JIT blocks do not read that clock.

Empirical gate: `crates/cli/tests/riscv_tick_interval_fidelity_differential.rs` — C3 OLED, tick 1 vs 64, **UART + inspect framebuffer identical**. Register snapshot at halt is **allowed** to differ (different micro-instant). That is the shipped definition of “tick widening does not change observable firmware.”

Thumb emit without an equivalent gate on a **real** Cortex-M UART/timer demo is not ready.

## 6. What landed vs what is still missing

**#1120 (`jit_framework/cortex_m/`)** is the emit path: ALU + in-window RAM load/store + common control flow, MMIO/`WIRE_MEM_FAULT` side-exit, WFI interpreter-owned, tick-512 ALU lockstep, UART MMIO store match.

**SysTick countdown clamp:** `CortexM::block_would_cross_irq` is the RISC-V `mtime` analogue. `Systick::ticks_until_fire` is the horizon; a compiled block of `n` cycles is refused when `n >= horizon` so exception 15 pends on the same instruction as the interpreter. After a block that does not wrap, `systick_consume_cycles` advances the legacy-walk CVR (scheduler-mode SysTick follows the cycle-clock bump). Dispatch still breaks the batch on an already-takeable exception.

**#1121 (`jit_framework/thumb/`)** was an all-bail walker only. Superseded by #1120 — do not merge both.

Still missing from the original “before emit” list:

- Firmware UART hello **JIT-on vs JIT-off** (Zephyr L0 / nRF) as a CI-weighted tick-interval gate (nRF differential covers hello at tick 512; a C3-style 1-vs-64 UART+inspect gate is the remaining analogue)

That proves dispatch/host/snapshot **plumbing**. It does **not**:

- run guest ALU in wasm
- handle RAM vs MMIO
- clamp SysTick/NVIC mid-block
- participate in `Machine::advance`

Calling it “DBT” or “as fast as tlib” is false. QEMU/tlib speed comes from **staying in translated RAM loops**. All-bail never does that.

## 7. Decision: Thumb emit?

**Not yet.** Required before any wasm emit for Thumb:

1. Write `block_would_cross_irq` for Cortex-M: SysTick countdown + NVIC already-pending, using the same “external lines stable within a batch” fact as RISC-V (`peripherals tick between batches`).
2. Copy the RAM-window + `MemoryFault` resume-at-faulting-PC path; do not inline MMIO.
3. Mirror `publish_cycle` / idle-ff / deadline clamp in the Cortex-M `step_batch` JIT loop (same shape as `RiscV::run_jit_loop`).
4. Add a **firmware** lockstep gate (not a 12-byte ADD loop): nRF or STM32 UART hello, JIT on vs off, same UART bytes — analogue of the C3 tick-interval test.
5. Keep `SafetyGate`; JIT off when observers/probes/breakpoints/cycle-accurate.

Until (1)–(4) exist **on paper as Cortex-M types and tests**, do not emit Thumb wasm.

**#1121** can stay as the walker + plumbing PR. Do not grow it into emit in the same change.

**#1119** (YAML sugar) is unrelated DX. It does not interact with this contract.

## 8. Sources (code)

- `crates/core/src/cpu/jit_framework/side_exit.rs`
- `crates/core/src/cpu/jit_framework/fallback.rs`
- `crates/core/src/cpu/jit_framework/riscv/emit.rs`
- `crates/core/src/cpu/riscv.rs` (`run_jit_loop`, `block_would_cross_irq`, `jit_gate_allows`)
- `crates/cli/tests/riscv_tick_interval_fidelity_differential.rs`

External: QEMU `docs/devel/tcg.html` (RAM vs MMIO), `docs/devel/tcg-icount.rst` (not cycle-accurate), Renode time framework (`PerformanceInMips`), tlib `cpu-exec.c` / `TranslationCPU.SyncTime`.
