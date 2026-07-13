# Universal Dispatch JIT — Framework Design (Speed Plan Phase 2)

Status: **design + scaffold** (this PR). No per-ISA codegen. Everything is
behind the `jit-framework` cargo feature; default builds and the existing
`jit` Xtensa pilot are untouched.

## 0. Context and non-goals

The Phase-3/4 Xtensa pilot (`crates/core/src/cpu/xtensa_jit/`,
`crates/wasm/src/jit_browser.rs`) proved that JIT-compiling hot basic
blocks to WebAssembly works on both native (`wasmtime`) and browser
(`js_sys::WebAssembly`). It hit two walls:

1. **It is Xtensa-specific and block-specific** — hand-picked PCs
   (`HOT_BB_PC = 0x400829cc`, `FILL_SCREEN_BLOCK_PC`), a bespoke walker,
   and per-block wasm bodies. It does not generalise to Cortex-M / RISC-V,
   nor to arbitrary hot blocks.
2. **A browser speed ceiling of ~1.5–2×** caused by dispatching every
   guest load/store through a **JS host import** (`host.read_u8` /
   `host.store_u8`). The JS⇄wasm boundary crossing — not the emitted
   arithmetic — dominated. See §5.

Phase 2 (this document) builds the **shared, ISA-agnostic framework** that
fixes the *architecture* problem and specifies the memory scheme that fixes
the *ceiling* problem. It explicitly does **not** implement any instruction
translation. Per-ISA frontends (Thumb-2 → RISC-V → Xtensa) and the real
`wasmtime` / `js_sys` runtime backends are later phases, gated on the
CPU-share measurement in §9.

The scaffold ships exactly one frontend — `PassthroughFrontend` — which
side-exits every block to the interpreter, and one runtime —
`InterpreterRuntime` — which always side-exits. Together they prove the
whole loop (cache → dispatch → instantiate → run → side-exit → fallback →
chaining) compiles and runs end-to-end with zero codegen.

Module map (`crates/core/src/cpu/jit_framework/`):

| Module | Responsibility |
| --- | --- |
| `mod.rs` | `Pc`, `CodeView`, `StateVec`, module docs |
| `block_cache.rs` | Flash-PC-keyed cache, hot-counter promotion, invalidate-all |
| `side_exit.rs` | `SideExit` protocol + `BailReason` |
| `frontend.rs` | `IsaFrontend` trait, `BlockPlan`, `PassthroughFrontend` |
| `runtime.rs` | `JitRuntime` trait, `MemoryBinding`, `InterpreterRuntime` |
| `fallback.rs` | `JitHost` interpreter hook, `SafetyGate` correctness rail |
| `dispatch.rs` | Chaining dispatch loop |
| `differential.rs` | Lockstep JIT-vs-interpreter equivalence harness |

## 1. Block cache

**Key: flash entry PC, and nothing else.** The JIT compiles only
flash-resident code. Flash is immutable after config (it is loaded once at
`load_firmware` and the `LinearMemory`/flash `Vec<u8>`s never reallocate),
so the mapping `pc → compiled block` is stable for a whole run. No PS/mode
bits in the key at the framework layer — an ISA that needs mode context
(e.g. Xtensa `CALLINC`) folds it into the frontend's decision to *refuse*,
not into the cache key. (The pilot's browser cache keyed by `(pc, ps_bits)`
as future-proofing; the universal cache keeps the key minimal per the
"simplest interface" rule and pushes mode-sensitivity into the frontend.)

**Hot-counter promotion.** A PC begins *cold*. Each dispatch landing on a
cold PC bumps a `u32` counter and interprets one instruction. When the
counter crosses `hot_threshold` (default `DEFAULT_HOT_THRESHOLD = 50`) the
PC is *promoted*: the frontend translates it and the compiled artifact is
installed. Rationale: one-shot init/boot code (thousands of PCs seen once)
never pays translation cost; only genuinely hot loops (the `invaders` main
loop, `fillScreen`, the delay loop) compile. The threshold is a tuning
knob measured against `invaders` in a later phase.

`BlockCache::observe(pc) -> Lookup` returns `Ready` (compiled, run it) or
`Interpret { promote }`, where `promote` is `true` exactly on the hit that
crosses the threshold — so the caller compiles once, not every subsequent
hit.

**Invalidation: invalidate-*all* on any flash write.** `invalidate_all()`
drops every entry (compiled artifacts *and* cold counters) and bumps a
generation counter. This is deliberately blunt:

- Flash writes are rare (OTA / self-update), so the cost is paid almost
  never.
- Because we compile only flash-resident code, a flash write is the *only*
  event that can invalidate emitted code. RAM writes never touch compiled
  bytes.
- Per-range invalidation would require a write-address → affected-block
  index (an inverted map maintained on every compile). Not worth the
  complexity for an event this rare. Correctness first.

The dispatcher polls `JitHost::take_flash_dirty()` at the top of each
iteration and calls `invalidate_all()` before the next lookup.

## 2. Dispatch

`DispatchLoop::run(host, budget)` is the engine. Each iteration:

1. **Flash poll** → `invalidate_all()` on a pending flash write.
2. **Safety gate** (§3) → if closed, interpret one instruction and loop.
3. **Cache `observe(pc)`**:
   - `Ready`: run the compiled artifact via the runtime, then **follow the
     side-exit**.
   - `Interpret { promote }`: if `promote`, translate + install; then
     interpret one instruction.

**Chaining dispatch.** When a block side-exits with `SideExit::Chain {
next_pc }`, the loop sets the guest PC to `next_pc` and continues. If
`next_pc` is itself hot, the *next* iteration runs its block with no
interpreter round-trip — block chains straight into block. This is the
throughput win: a hot loop body chaining to its own head runs as
back-to-back compiled blocks. (A later phase can add direct block→block
tail-call links inside the emitted wasm to skip even the dispatch-loop
round-trip; the `Chain` protocol is the seam for that.)

### Side-exit protocol (`side_exit.rs`)

A compiled block never runs unbounded. It executes a translated run of
instructions and returns one `SideExit`:

- `Chain { next_pc }` — ran to the terminator; control flows to `next_pc`
  (fall-through or a fully-modeled taken branch).
- `EnterInterpreter { resume_pc, reason }` — bail to the interpreter for at
  least one instruction. Reasons (`BailReason`): `UnsupportedInstruction`
  (opcode the frontend doesn't translate), `PartialBlock` (block was cut at
  a non-compiled region), `MemoryFault` (load/store resolved to
  MMIO/peripheral space outside the imported window — replay on the real
  `Bus`), `SafetyGate` (a rail tripped mid-block), `Passthrough` (scaffold).
- `Exception { resume_pc, cause }` — a synchronous exception / taken
  interrupt fired inside the block; effects are unwound to the faulting
  instruction and the machine's exception path vectors from `resume_pc`.

The enum is the ISA-neutral vocabulary. Per-ISA emit backends encode a
matching `i32` **wire code** inside the wasm body (the Xtensa pilot's
`EXIT_FALL_THROUGH = 0`, `EXIT_HOST_BUS_ERROR = 5` in `xtensa_jit_bytes`);
the runtime adapter maps wire code → `SideExit`. `BlockPlan::exits` carries
the static `wire_code → BailReason` map per block.

## 3. Correctness rails — when the JIT must NOT run

The interpreter is the single source of truth; the JIT is a bulk fast-path
layered on top. A compiled block retires many instructions as one host
call: it advances the cycle counter in a lump and never fires
`on_step_start` / `on_memory_write` per instruction. That is invisible to a
plain run but **wrong** the instant something needs per-instruction or
per-cycle granularity. So `SafetyGate::jit_allowed()` must be `true` for the
JIT to run, and it is `false` whenever any of these is active. The gate is
recomputed **every dispatch iteration**, so toggling mid-run (a debugger
attaches, a probe arms) takes effect on the very next instruction.

| Rail | Detected by (on the real `Machine`) | Why the JIT is unsafe |
| --- | --- | --- |
| **Observers** | `!observers.is_empty()` (the `&[Arc<dyn SimulationObserver>]` handed to `Cpu::step`) | Observers want `on_step_start`/`on_step_end`/`on_memory_write` per instruction; a block elides them. |
| **Breakpoints** | machine breakpoint set non-empty (`DebugControl`) | A block could step over a breakpoint address without stopping. |
| **Probes** | any logic-analyzer / signal tap armed (`bus.logic_tap().is_some()`, DAP watch, `push_armed()`) | These need per-cycle pad/edge visibility the JIT does not model. |
| **Cycle-accurate mode** | cycle-accurate mode selected (vs. batched throughput mode; cf. `SimulationConfig`) | The JIT models retirement, not per-cycle timing. |

**How a running block bails.** Two layers. (a) *Entry*: the dispatch loop
checks `jit_allowed()` before ever consulting the cache — if a rail is
active it interprets and never enters a block. (b) *Mid-flight*: if a rail
can arm asynchronously during a long block, the emitted body checks the
armed flag at its safe points (e.g. before a memory op) and returns
`EnterInterpreter { reason: SafetyGate }`; the block's effects up to that
point are already committed to the shared memory, and the interpreter
resumes at `resume_pc`. The *next* dispatch entry sees `jit_allowed() ==
false` and stays on the interpreter until the rail clears. Since compiled
blocks are short (a basic block), (a) is sufficient for every rail that can
only change at API boundaries; (b) is the belt-and-braces for taps that arm
from another thread.

## 4. Memory — the imported-memory scheme (the ceiling fix)

The pilot dispatched each guest memory access through a JS host import.
That is what capped browser speedup at ~1.5–2×: the JS⇄wasm boundary
crossing per load/store dwarfed the arithmetic. The fix is `MemoryBinding`:
**emitted blocks import the engine's own backing memory and issue plain
`i32.load` / `i32.store`** against it — no host call on the hot path.

- **Browser (`js_sys::WebAssembly`).** The engine's `LinearMemory` (RAM)
  and flash `Vec<u8>`s are exposed as a single `WebAssembly.Memory`,
  imported into every emitted module. A guest load of `addr` becomes one
  wasm memory op at offset `addr - guest_base`. This is sound because the
  `LinearMemory.data` / flash `Vec`s **never reallocate after config** — the
  base the emitter bakes in stays valid for the whole run, and the block
  cache is dropped on the (rare) flash write anyway (§1). This is the change
  that removes the 1.5–2× ceiling: the dominant cost on `invaders` was the
  per-access import, not codegen quality.
- **Native (`wasmtime`).** `wasmtime` maps the same region as linear memory
  (or the runtime resolves a raw base pointer into the `LinearMemory` `Vec`
  at instantiate time). Same emitted op, no host trampoline.

`MemoryBinding` (`NativeLinear { guest_base, len }` /
`BrowserSharedMemory { guest_base, len, region }`) is intentionally
pointer-free at the framework layer so it stays `Send + Sync` and drags in
no `js_sys`/`wasmtime` types; the concrete runtime resolves the actual
pointer / `Memory` object internally.

**MMIO stays on the Bus.** Peripheral / MMIO addresses are *not* in the
imported window. The emitter range-checks each access against the RAM/flash
window (`MemoryBinding::contains`) and emits `EnterInterpreter { reason:
MemoryFault }` for anything outside it, so peripheral reads/writes still go
through the real `Bus` (peripherals, traps, observers). The imported-memory
fast path is exactly and only for plain RAM/flash — which is the
overwhelming majority of hot-loop accesses.

## 5. Runtimes — native vs browser behind one interface

`JitRuntime` abstracts the execution engine:

```
trait JitRuntime {
    type Artifact: JitArtifact;
    fn backend_name(&self) -> &'static str;
    fn instantiate(&mut self, plan: &BlockPlan, mem: &MemoryBinding)
        -> Result<Self::Artifact, RuntimeError>;
    fn run(&mut self, artifact: &mut Self::Artifact) -> SideExit;
}
```

- **`NativeWasmtimeRuntime`** (later phase) — `wasmtime::{Engine, Module,
  Instance}`; `Artifact` wraps a `Store` + `TypedFunc`. `wasmtime` does not
  build for `wasm32-unknown-unknown`, so this is `#[cfg(feature = "jit")]`
  / non-wasm only.
- **`BrowserWebAssemblyRuntime`** (later phase) — `js_sys::WebAssembly::{
  Module, Instance}`; `Artifact` caches the exported `run` `Function` for a
  direct `callN`. `wasm32` targets only.
- **`InterpreterRuntime`** (this scaffold, always available) — no codegen;
  `run` always returns `EnterInterpreter`. It is both the passthrough
  proof-of-loop and the honest fallback for build targets where neither
  wasm engine is present.

The `BlockPlan.code` byte stream is identical across native and browser —
translation (frontend) is fully decoupled from execution (runtime), exactly
as the pilot's `emit_core` / `xtensa_jit_bytes` split already established.
Register marshalling (guest register file ⇄ block params/results) is the
runtime's concern; the scaffold's `run` signature elides it and the real
runtimes take a register-file handle there.

## 6. Interpreter fallback hook

The dispatch loop drives the machine **entirely** through `JitHost`
(`fallback.rs`), never touching a concrete `Cpu`/`Bus`:

```
trait JitHost {
    fn pc(&self) -> Pc;
    fn interpret_one(&mut self) -> HostStep;      // the universal fallback
    fn resume_at(&mut self, pc: Pc);
    fn code_view(&self, pc: Pc) -> Option<CodeView<'_>>;  // Bus::fetch_slice
    fn safety(&self) -> SafetyGate;
    fn snapshot_state(&self) -> StateVec;         // for the diff harness
    fn take_flash_dirty(&mut self) -> bool;
}
```

The eventual adapter implements `JitHost` for `Machine<C>`: `interpret_one`
= one `Cpu::step`, `code_view` = `Bus::fetch_slice`, `safety` = poll
observers/breakpoints/taps/mode. Every uncompiled instruction, partial
block, memory fault, exception, and tripped rail routes to
`interpret_one` — the interpreter remains the correctness reference.

## 7. Differential harness — the merge gate's proof (`differential.rs`)

Run the same firmware twice from the same reset — JIT-enabled and pure
interpreter — and assert architectural state matches at every comparison
point. Any divergence is a JIT bug, reported with the exact step and the
first differing `StateVec` word (`Divergence { at_step, word_index,
interp, jit }`).

- **Cadence.** Comparing every instruction is strongest but O(state)/step.
  Because a block retires many instructions atomically, the natural cadence
  is **per compiled-block boundary** (compare when the JIT side-exits),
  with an optional per-N-instruction cap for long straight-line runs.
  `DiffPolicy.block_boundary_only` selects this. The two sides are only ever
  compared when known PC-aligned.
- **Masking.** `DiffPolicy.ignore_indices` masks words that legitimately
  differ between a batched and a per-instruction run (e.g. a free-running
  cycle counter sampled mid-block) — mirroring the pilot's
  `ComparePolicy` CCOUNT tolerance in `xtensa_lockstep`.
- **Reuse.** The Xtensa pilot already ships a concrete, richer version
  (`xtensa_lockstep::{LockstepRunner, compare_traces, ComparePolicy}`,
  `tests/jit_lockstep.rs`). As the framework absorbs each ISA, that logic
  generalises onto this `StateVec`-based interface; `DifferentialHarness`'s
  factory-closure shape matches `LockstepRunner::new`.

This harness is a **merge gate**: no per-ISA frontend merges without a
green differential run on that ISA's reference firmware (`invaders` for
Xtensa, an equivalent hot-loop firmware per ISA).

## 8. Per-ISA frontend trait and order

```
trait IsaFrontend {
    fn isa_name(&self) -> &'static str;
    fn translate_block(&self, pc: Pc, code: &CodeView<'_>)
        -> Result<BlockPlan, FrontendRefusal>;
}
```

`translate_block` walks a basic block from `pc` over the flash `CodeView`
and emits a `BlockPlan { entry_pc, end_pc, instr_count, code, exits }`, or
refuses (never an error — the PC just stays on the interpreter). Contract:
compile only flash-resident, position-stable code; terminate at the first
fully-modeled control-flow instruction, or cut the block at the first
unmodeled instruction (emitting an `EnterInterpreter` edge there).

**Order of implementation and why:**

1. **Thumb-2 (Cortex-M)** — first. Widest board coverage in LabWired
   (STM32 F1/F4/H5/L0/L4, nRF52/53, RP2040, KW41Z all share the core).
   Clean 16/32-bit encoding, no register windows, no literal-pool
   indirection. Simplest emit, biggest immediate payoff, and the emit is
   reused verbatim across every Cortex-M board.
2. **RISC-V (RV32IMC, ESP32-C3)** — second. Regular fixed encoding, no
   windows. Validates the framework on a structurally different ISA and
   unlocks the C3 (a live proto.cat/LabWired target).
3. **Xtensa (LX7, ESP32-S3/classic)** — last, hardest. Register windows
   (`CALL8`/`RETW`, `CALLINC`), the `L32R` literal pool, density
   instructions. The existing pilot (`xtensa_jit`, `jit_browser`) is
   absorbed here as the reference frontend once the framework is proven on
   the two easier ISAs.

## 9. Merge bar and entry gate

**Entry gate (when Phase 2 codegen is allowed to start).** Per-ISA
translation work is gated on a **CPU-share measurement**: the walk-free
campaign must establish that the CPU interpreter is **~90%** of simulation
wall-time on the target firmware. If peripheral modeling or bus dispatch
dominates instead, JIT-ing the CPU is premature — fix that first. The
scaffold in this PR is the only JIT work that lands before that gate; it
adds no codegen and changes no default behavior.

**Merge bar (when a frontend is allowed to merge).** Each per-ISA frontend
must clear **≥ 3× interpreter MIPS on `invaders`** (the reference hot-loop
firmware), *and* pass a green differential run (§7).

**Measurement method.**

- Same ELF, same reset state, same input trace, deterministic run.
- Metric: retired guest instructions per second (MIPS) over the steady-state
  hot region (exclude boot/init), interpreter build vs. `jit-framework`
  build, native and browser measured separately.
- Report `RunStats` alongside: `block_runs`, `chained`, `interpreted`,
  compiled-block coverage (fraction of retired instructions executed inside
  compiled blocks). A speedup with low coverage means the win is fragile.
- Browser number is the one that matters for the product ceiling — that is
  where the imported-memory scheme (§4) must be shown to beat the pilot's
  1.5–2×. Baseline to beat: the pilot's ~1.05 edge-rate / 1.5–2× browser
  speedup.

## 10. What ships in this PR (scaffold)

- The framework modules above, behind `jit-framework`.
- `PassthroughFrontend` (side-exits every block) + `InterpreterRuntime`
  (always side-exits): the full loop runs end-to-end with **zero** codegen.
- Unit tests: block-cache promotion + invalidate-all-on-flash-write;
  memory-window check; interpreter-runtime side-exit; end-to-end passthrough
  dispatch (program runs to completion via fallback); safety-gate forces
  pure interpretation; flash-write invalidation mid-run; differential
  compare + harness.
- **No** Thumb-2 / RISC-V / Xtensa instruction translation. **No** changes
  to the esp32c3 peripheral files, `EXPECTED_PINNERS`, or the
  walk-differential tests (owned by a concurrent campaign).
