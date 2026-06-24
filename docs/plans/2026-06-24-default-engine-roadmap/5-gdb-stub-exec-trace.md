# Plan #5 — Debug surface: GDB remote stub hardening + execution trace

Internal implementation plan. No code changes here. Status of cited code verified against
clean `origin/main` checkout (tip `a876d471`) at `…/scratchpad/wt-main`. All line numbers below
are from THAT checkout. This supersedes a prior draft that read a branch ~314 commits behind main;
corrections vs that stale draft are called out inline as **[CORRECTION]**.

## 1. Ground truth (what already exists)

This is EXTEND-AND-HARDEN, not greenfield.

GDB RSP stub — `crates/gdbstub/src/lib.rs`:
- `LabwiredTarget<C>` wraps `Machine<C>`. Implements the `gdbstub` crate's `Target` for both
  `CortexM` (`Armv4t` arch, lib.rs:36-49) and `RiscV` (`Riscv32`, lib.rs:104-117).
- `SingleThreadBase`: register read/write `g`/`G` (cortex_m lib.rs:51-78 maps r0-r12/sp/lr/pc/xPSR;
  riscv lib.rs:119-140 maps x0-x31/pc), memory `m`/`M` (lib.rs:80-95, 142-165 via
  `Machine::read_memory`/`write_memory`).
- `SingleThreadResume`/`SingleThreadSingleStep`: `c`/`s` (lib.rs:174-200) — these only set
  `running`/`single_step` flags; the actual stepping happens in the event loop.
- `SwBreakpoint`: `Z0`/`z0` (lib.rs:213-234) forwarding to `Machine::add_breakpoint`/`remove_breakpoint`.
- `GdbEventLoop::wait_for_stop_reason` (lib.rs:285-355) is the run loop: non-blocking peek for an
  inbound interrupt byte, else `step_single()` (single-step) or `machine.run(Some(1000))` (continue),
  mapping `StopReason::Breakpoint`/`StepDone` -> SIGTRAP, errors -> SIGSEGV. `on_interrupt` -> SIGINT (lib.rs:357).
- Transport: `GdbServer` (lib.rs:236-272) binds `0.0.0.0:<port>`, accepts ONE TCP client, blocking.
- E2E socket test `crates/gdbstub/tests/gdb_e2e.rs` drives `g`, `s`, `m`, ctrl-C(0x03)->SIGINT over a
  real socket against `uart-ok-thumbv7m.elf`.

CLI wiring — `--gdb <port>` IS already a flag (`crates/cli/src/main.rs:85-87`, `Option<u16>`), consumed in
`crates/cli/src/commands/run.rs`: CortexM (run.rs:1054-1061) and RiscV (run.rs:1123-1130) construct
`GdbServer::new(port).run(machine)`; Xtensa explicitly errors "not yet supported" (run.rs:1185-1188).

Observer / trace infrastructure:
- `SimulationObserver` trait — `crates/core/src/lib.rs:148-155`: `on_simulation_start/stop`,
  `on_step_start(pc, opcode)`, `on_step_end(cycles, &registers)`, `on_memory_write(addr, old, new)`,
  `on_peripheral_tick(name, cycles)`. Fanned out per-instruction in the CPU step impls:
  cortex_m `on_step_start` at cpu/cortex_m.rs:859 (guarded by `if !_observers.is_empty()`),
  `on_step_end` at cpu/cortex_m.rs:2785; riscv `on_step_start` cpu/riscv.rs:160, `on_step_end` cpu/riscv.rs:740.
- `TraceObserver` + `InstructionTrace` exist in `crates/core/src/trace.rs` (pc, instruction, cycle,
  register_delta, memory_writes, mnemonic, stack_depth, function).
- `crates/hw-trace/src/event.rs` is the `TraceEvent::Placeholder` enum to fill ("Populated fully in Plan 2").

### Corrections vs the stale-branch draft
- **[CORRECTION] An exec-trace CLI flag DOES already exist.** `crates/cli/src/main.rs:77-79` defines
  `--trace` (global), and `commands` build a `TraceObserver` and write `trace.json` (main.rs:1521-1522 construct;
  main.rs:1931-1941 serialize via `obs.take_traces()`). The stale draft's "nothing constructs/emits
  InstructionTrace; only VCD is wired" is WRONG on main. So Plan-2's job is not "add the first trace flag"
  but: (a) make the trace event-source the SAME hook coverage consumes, (b) add a compact streaming format
  (today it's a JSON array buffered in RAM, capped by `--trace-max`, main.rs:586), (c) make it ~zero overhead
  when off.
- **[CORRECTION] The DAP crate already consumes `InstructionTrace`** (`crates/dap/src/adapter.rs:9,432,570`,
  `crates/dap/src/server.rs:214,1464` `readInstructionTrace`). Any refactor of `trace.rs`/observer signatures
  is a THIRD consumer to keep green, not just CLI — the stale draft missed this.
- **[CORRECTION] `on_step_start` IS already observer-gated** in cortex_m (cpu/cortex_m.rs:859), but
  `on_step_end` is NOT — see §2.3. The stale draft framed the whole snapshot as ungated; only the end half is.

## 2. Verified real gaps to target

### 2.1 Breakpoints unreliable under `continue` (PRIMARY bug)
`Machine::run` (`crates/core/src/lib.rs:1323-1422`) checks breakpoints ONLY at batch boundaries
(lib.rs:1327-1334, before each `step_batch`). It then calls `cpu.step_batch(.., current_batch)`
(lib.rs:1371-1373) where `current_batch` = `remaining_until_tick` (lib.rs:1352), i.e. up to a full
`peripheral_tick_interval` of instructions (hundreds–thousands), clamped to 1 ONLY for cycle-accurate
buses (lib.rs:1367-1369). The default `step_batch` (lib.rs:174-185, cortex_m override cpu/cortex_m.rs:629-690)
just loops `step()` `max_count` times with NO breakpoint awareness. Result: a breakpoint whose PC is hit
*inside* a batch is executed straight past; `run` only notices it on the NEXT batch boundary (PC already
moved on) — so a BP in the middle of a hot basic block is silently skipped. Under GDB this is the classic
"set breakpoint, continue, never stops" symptom.

`last_breakpoint` (lib.rs:681,1331-1337) is a sticky de-dupe so a BP at the current PC doesn't re-fire
immediately on resume — that logic is fine and must be preserved.

### 2.2 PC coverage source does not exist; `coverage/` is unrelated
`crates/core/src/coverage/` (mod.rs, probe.rs) is SVD-driven *register-faithfulness* probing
("measures by OBSERVABLE BEHAVIOR whether a model implements each register a chip's SVD declares",
coverage/mod.rs header). It is NOT runtime PC/branch coverage. Plan item #2 (coverage) needs a runtime
PC-stream source that does not exist yet — and that source is exactly the trace event hook from this plan.

### 2.3 Per-instruction register snapshot runs even with zero observers (perf)
cortex_m builds a 17-`u32` register array EVERY instruction unconditionally
(`crates/core/src/cpu/cortex_m.rs:2779-2787`) before the `for obs in _observers` fan-out — the build is
OUTSIDE any `is_empty()` guard. riscv does the same with a 33-`u32` array (`crates/core/src/cpu/riscv.rs:736-742`).
With no observers attached this is pure waste on the hot path (17/33 `get_register` calls + copies per insn).

### 2.4 GDB stub bypasses the observer fan-out
The stub drives `Machine::run`/`step_single`, which fan observers out normally — BUT the stub itself does not
register any observer, so a GDB session produces no trace/coverage events. Wiring trace+coverage sinks to be
active under `--gdb` (so "debug + collect coverage in one run") is part of the cross-plan unification.

### 2.5 Packet-coverage gaps (audit)
Present: `g/G`, `m/M`, `c`, `s`, `Z0/z0`, ctrl-C/SIGINT. Missing / to confirm against a real
`arm-none-eabi-gdb` handshake:
- `vCont`/`vCont?` — gdbstub advertises capabilities; modern gdb prefers vCont for `c`/`s`. Verify the
  `gdbstub` crate version negotiates this for us or whether `c`/`s` fallback is what gdb actually uses.
- Target description XML (`qXfer:features:read`) — `Armv4t` is the wrong register layout for Cortex-M
  (no FPU/CONTROL/PRIMASK; xPSR mapped onto `cpsr`). Works for basic g/G but `info registers` will mislabel.
  Decide: keep Armv4t (cheap, lossy) vs custom target.xml (correct, more work) — see §3.1 step 4 / Phase B.
- `?` (initial stop reason): on connect with `running=false` the loop returns SIGTRAP immediately
  (lib.rs:298-302) — confirm gdb is happy with that as the first stop.
- Single-client only — a second `gdb target remote` while one is attached hangs on accept (lib.rs:259).

## 3. Cross-plan design — ONE event-emission hook (shared with Plan #2)

Promote `SimulationObserver` into a typed `TraceEvent` source feeding three sinks. This is the single
contract Plan #2 (coverage) and this plan (gdb stop-control + exec-trace) both consume.

Fill `crates/hw-trace/src/event.rs` `TraceEvent` (replacing `Placeholder`):

    enum TraceEvent {
        StepStart { pc: u32, opcode: u32 },
        StepEnd   { pc: u32, cycles: u32, regs: RegView },   // RegView = borrowed/lazy, see perf
        MemWrite  { addr: u64, old: u8, new: u8 },
        Irq       { vector: u32 },           // emitted where set_exception_pending fires (lib.rs:1390)
        PeripheralTick { name, cycles: u32 },
    }

Three sinks, all behind the existing `Vec<Arc<dyn SimulationObserver>>` fan-out (no new dispatch path):
1. GDB stop-controller — owns breakpoint matching so `continue` stops mid-batch (Phase A).
2. Exec-trace writer — compact streaming format (Phase C), replaces/extends `TraceObserver`.
3. Coverage collector — Plan #2 owns this; consumes `StepStart.pc` for PC/branch coverage.

Zero-overhead-when-disabled: cache an OR'd `EventMask` (bitset of which event kinds ANY registered sink
wants), recomputed when observers change. The CPU step path checks one `mask & KIND != 0` branch before
doing any work — crucially gating the §2.3 register-array build (build `RegView` only if any sink wants
`StepEnd` regs). Keep the existing `SimulationObserver` trait as the transport (DAP + VCD + metrics already
implement it) and ADD a default `event_mask()` (or `wants(kind)`), so existing impls are untouched and
report "want everything" by default to preserve current behavior; new high-volume sinks (trace/coverage)
declare a narrow mask.

## 4. Workstreams, phases, files touched

### Phase A — fix breakpoints under `continue` (highest value, smallest blast radius)
Make `Machine::run` stop AT the breakpoint PC, not at the next batch boundary.
- Approach: when `self.breakpoints` is non-empty, clamp `current_batch` to 1 (mirror the existing
  cycle-accurate clamp at `crates/core/src/lib.rs:1367-1369`) so the BP check at lib.rs:1327-1334 runs
  before every instruction. Cost is per-instruction batching ONLY while breakpoints are set (debugging),
  so the no-BP hot path is unaffected.
  - Cleaner alternative (more work): make `step_batch` breakpoint-aware — pass the breakpoint set down
    and let it return early with the count executed when PC matches. Heavier (touches the `Cpu` trait
    signature `crates/core/src/lib.rs:174-185` + both CPU overrides + `Box<dyn Cpu>` forward lib.rs:280-288).
    Defer unless the clamp's debug-mode slowdown is unacceptable.
- Files: `crates/core/src/lib.rs` (`run`, ~1346-1369). Preserve `last_breakpoint` sticky logic.
- Tests: extend `crates/gdbstub/tests/gdb_e2e.rs` — set `Z0` at a PC several instructions into a basic
  block, `c`, assert SIGTRAP with PC == breakpoint (today it would run past). Add a `core` unit test on
  `Machine::run` directly: BP mid-batch stops exactly at the BP PC.

### Phase B — GDB RSP harden + packet audit
- Drive a real `arm-none-eabi-gdb` against `labwired run --gdb <port>`; script `break`, `continue`, `step`,
  `info registers`, `x/4xw`, `set var`, ctrl-C. Capture the RSP packet log (`set debug remote 1`).
- Decide Armv4t-vs-custom-target.xml (§2.5). If keeping Armv4t, document the xPSR/cpsr aliasing limitation.
- Confirm `vCont`/`?`/reconnect behavior; add a friendly log when a 2nd client is refused (or queue).
- Wire trace + coverage sinks so they're active under `--gdb` (§2.4): the stop-controller sink owns BP
  matching; trace/coverage sinks attach if their flags are also passed.
- Files: `crates/gdbstub/src/lib.rs` (target XML, event-loop sink registration, transport log),
  `crates/cli/src/commands/run.rs:1054-1061,1123-1130` (pass trace/coverage sinks into `GdbServer::run`).
- Tests: extend `gdb_e2e.rs` for `Z0`+`c` stop (shared with Phase A) and a register round-trip via `G`.

### Phase C — exec-trace event stream + compact format
- Fill `crates/hw-trace/src/event.rs` `TraceEvent` (§3). Add an `EventMask` and the
  `SimulationObserver::event_mask()` default.
- Gate the §2.3 register-array build behind the mask in both CPUs
  (`crates/core/src/cpu/cortex_m.rs:2779-2787`, `crates/core/src/cpu/riscv.rs:736-742`).
- Add a streaming `ExecTraceWriter` sink (compact, append-only): suggest a fixed-width binary record
  (pc:u32, opcode:u32, cycle-delta varint, optional mem/irq sub-records) or length-prefixed framing, written
  to a file/pipe rather than buffered like today's `TraceObserver` JSON array (`crates/core/src/trace.rs`,
  capped by `--trace-max`). Keep `trace.json` as an opt-in pretty mode for back-compat with DAP.
- Flag: keep `--trace` (main.rs:77-79); add `--trace-format {json,compact}` and `--trace-out <path>`.
  Default compact-to-file for long runs; `--trace` alone keeps today's `trace.json`.
- Files: `crates/hw-trace/src/event.rs`, `crates/hw-trace/src/lib.rs`, `crates/core/src/trace.rs`
  (add streaming sink or new module), `crates/core/src/cpu/{cortex_m.rs,riscv.rs}` (mask gate),
  `crates/core/src/lib.rs` (emit `Irq` at lib.rs:1390; `event_mask` recompute when `observers` change),
  `crates/cli/src/main.rs` + `commands/run.rs` (flags + sink construction).
- Keep GREEN: `crates/dap/src/adapter.rs` + `server.rs` still need `InstructionTrace` — don't break its shape.

## 5. Perf
- No-BP, no-trace continue path: unchanged batch size; the ONLY new cost is recomputing `event_mask` when
  observers change (rare) and one `mask` branch per instruction. The §2.3 fix REMOVES per-insn register-array
  builds when no sink wants StepEnd regs — net win on the default path.
- Debug path (BP set): batch clamps to 1 — slower, but only while attached/debugging. Acceptable; this is the
  same trade the cycle-accurate buses already make (lib.rs:1367-1369).
- Compact streaming trace avoids the unbounded in-RAM `Vec<InstructionTrace>` growth (today bounded only by
  `--trace-max`); write-behind to a file keeps steady-state memory flat.

## 6. Risks
- Trait-signature churn: adding `event_mask()` as a defaulted method avoids breaking the existing
  `SimulationObserver` impls (TraceObserver, VcdObserver, PerformanceMetrics/metrics.rs, gpio_observer, DAP).
  Verify each compiles and that defaulting to "wants everything" preserves current VCD/DAP/metrics behavior.
- Armv4t target description is lossy for Cortex-M; switching to custom target.xml risks breaking the existing
  e2e RLE-`g` parsing assumptions (gdb_e2e.rs:152-197 expects 16x8 hex). Coordinate the test if layout changes.
- BP clamp-to-1 must NOT change semantics of `MaxStepsReached`/tick cadence: re-verify the `remaining_until_tick`
  and `total_cycles % tick == 0` peripheral propagation (lib.rs:1378-1393) still fires correctly at batch=1.
- gdbstub-crate version: confirm which RSP features (vCont, qXfer) it negotiates before promising them.

## 7. Explicitly deferred
Reverse-debug / checkpointing (snapshot infra exists at `crates/core/src/snapshot.rs` but time-travel is out
of scope), hardware watchpoints (`Z2/Z3/Z4`), Xtensa GDB (`crates/cli/src/commands/run.rs:1185-1188` stays an
error), multi-client GDB.
