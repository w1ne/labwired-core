# LabWired "what incumbents have / what we build next" — consolidated roadmap

INTERNAL. Private super repo only — never the public core repo (standing rule). 2026-06-24.

Five sub-plans in this dir, one per ranked item. This file ties them together:
sequencing and cross-plan dependencies. All grounded on core origin/main tip a876d471.

## GROUNDING (all plans re-verified on origin/main)

All five sub-plans were RE-RUN against a clean `origin/main` checkout (worktree at
scratchpad/wt-main, tip `a876d471`, PR #353 benchmark is an ancestor). Every claim cites
file:line on that checkout. The earlier stale-branch draft (read `feat/cli-ingest-svd`,
314 behind main) is fully superseded. Corrections folded in:

- CI composite action `.github/actions/labwired-test/` EXISTS → plan 4 now EXTENDS it
  (refactor install block into sibling `labwired-install`, add `labwired-robot`/`labwired-behave`).
- `examples/f103-fidelity-bench/` EXISTS → plans 1 & 3 build on the real 4/4 bench.
- `clock:` field EXISTS (`ClockGate{reg,bit}`, config/src/lib.rs:90-96) AND clock-gating is
  bus-level/peripheral-agnostic (`is_peripheral_clocked`, bus/mod.rs:1216) → unclocked-reads-0
  already works for GenericPeripheral; `missing_clock` fault lowers onto dropping a resolved
  clock_gate (NOT deferred). Only the RCC producer side stays Rust.
- `--trace` exec-trace flag ALREADY exists (TraceObserver → trace.json); DAP crate is a third
  InstructionTrace consumer → plan 5 keeps it green.
- batch mode does NOT bypass observers (step_batch → step_internal per-insn) → no kill-switch.
- GDB stub doesn't bypass the step path, it just never attaches an observer → one-line fix.
- `labwired coverage` verb already exists (SVD register-faithfulness probe, NOT PC coverage)
  → firmware PC coverage is a flag on test/run, not that verb.
- MemoryViolation does NOT vector into a HardFault handler (aborts sim / silently swallowed)
  → handler-vectoring is a real phased CPU task (plan 3 Phase 3).

## The five items (ranked by leverage on the wedge per effort)

1. Declarative peripheral breadth  — `1-declarative-peripheral-breadth.md`
2. Cert / coverage evidence output — `2-cert-coverage-evidence.md`
3. Fault injection (first-class)   — `3-fault-injection.md`
4. Robot + Cucumber runners        — `4-robot-cucumber-runners.md`
5. GDB stub + execution trace      — `5-gdb-stub-exec-trace.md`

## Cross-plan dependencies (the important part)

- **Shared event source (foundation).** Plans 2 and 5 INDEPENDENTLY converged on the
  same hook: promote `SimulationObserver` (core/src/lib.rs:148, fanned out per-step in
  cortex_m.rs:699 / riscv.rs) into one typed `TraceEvent` stream (fills the `hw-trace`
  placeholder). Three sinks read it: GDB stop-controller, exec-trace writer, coverage
  collector. Build this ONCE, first. Both plans note the GDB stub currently bypasses the
  observer and must attach it for coverage runs.
- **#2 consumes #3 and #5.** Cert report = coverage (#2) + fault-injection evidence (#3's
  per-fault `fault_triggered`/verdict record) + trace (#5). Fix the evidence-record
  contract early so #3 emits what #2 ingests.
- **#3 builds on #1.** Fault kinds lower onto GenericPeripheral side-effect/timing
  descriptors (#1's substrate). `missing_clock` rides the `clock:` field (exists on main).
- **#1 is independent** — top-leverage, no blockers, can start immediately.
- **#4 is independent** — thin wrappers over the `labwired test` CLI contract; lowest
  risk; can run anytime in parallel.

## Suggested sequence

0. Foundation: unified `TraceEvent` observer hook (small; unblocks #2 + #5).
1. In parallel from day one:
   - #1 declarative breadth (the moat; independent).
   - #4 Robot + Cucumber wrappers (independent; cheap adoption win).
2. #5 GDB-harden + exec-trace (on the foundation).
3. #2 coverage + signable run-manifest (on the foundation; consumes #5 trace).
4. #3 fault injection (on #1; emits evidence into #2).

This ordering keeps the two moat items (#1, #2/#3) central, gets an adoption win (#4)
landing early and cheap, and builds the shared hook once instead of three times.

## Strategy framing (unchanged)

Don't out-breadth or out-feature the incumbents (co-sim/RTL, reverse-debug, Linux/MMU,
mobile, large wireless-net = real gaps, wrong fights). Close JUST enough breadth (#1) +
adoption (#4) to be adoptable; sell fidelity / cert-grade evidence (#2/#3) where the
free-but-low-fidelity and the expensive-incumbent tools can't follow. Beachhead decision
(safety-cert CI vs broad "default engine") still OPEN — it only re-orders #1/#4 vs #2/#3,
not the contents.
