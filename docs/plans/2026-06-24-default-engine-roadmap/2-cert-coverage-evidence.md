# Plan #2 — Certification / coverage evidence output

INTERNAL implementation plan. No code changes performed; this is a design + sequencing
document. All citations are against the clean `origin/main` checkout at tip
`a876d471` (read-only worktree). Re-verified against THIS checkout — see "Corrections vs
the stale-branch draft" at the end; the stale draft was 314 commits behind and several of
its premises were wrong.

## 1. What the buyer is paying for

A safety-critical integrator (ISO 26262 / DO-178C / IEC 61508 toolchain qualification or
verification credit) needs three artifacts out of a single `labwired test` run, all tied to
the *unmodified* firmware ELF they ship:

1. **Structured statement + branch coverage** mapped to the ELF's DWARF line/branch info —
   the raw material for a statement/branch coverage claim, the floor under MC-DC.
2. **A deterministic, signable `RunManifest`** — ELF hash, chip-model version, config
   hashes, engine version, seed, results, and a canonical-JSON digest with wall-clock time
   EXCLUDED, so two runs on two machines produce a byte-identical digest.
3. **A cert-suitable report bundle** — LCOV (`.info`) + structured JSON + the existing
   JUnit, so it drops into qualification tooling and CI dashboards unchanged.

The wedge is *provable* evidence: the coverage number must come from observed PC flow, the
manifest must be bit-reproducible and signable, and the whole bundle must be auditable
without trusting our prose.

## 2. Ground truth in the current code (file:line)

### 2.1 The run loop and existing outputs
- `crates/cli/src/main.rs:1500` `execute_test_loop` — the test driver. It pushes optional
  observers onto `machine.observers` (`TraceObserver` at `:1523`, `VcdObserver` at `:1532`),
  computes `batch_size` (10000 when batchable, else 1) at `:1539`, and runs either
  `cpu.step_batch(&mut machine.bus, &machine.observers, &machine.config, to_execute)` at
  `:1588` or `machine.step()` at `:1686`.
- `crates/cli/src/main.rs:1874` `write_outputs` — SHA-256s `firmware_bytes` at `:1891-1893`,
  builds `TestResult` (`:1896`, struct defined at `:594`), and writes `result.json`
  (`:1921`), `trace.json` (`:1933`, only when `--trace`), `snapshot.json` (`:1946`),
  `uart.log` (`:1972`), and `junit.xml` via `write_junit_xml` (`:1980`).
- `write_junit_xml` at `crates/cli/src/main.rs:2187` already emits firmware_hash, config,
  limits, stop-reason, and per-assertion `<testcase>` rows; schema version is the const
  `RESULT_SCHEMA_VERSION = "1.0"` at `:38`, surfaced as a JUnit `<property>` at `:2342`.

So the run already SHA-256s the ELF and emits result/junit/uart/snapshot. We extend this
path; we do not rebuild it.

### 2.2 The observer trait and its fan-out (the capture hook)
- Trait: `crates/core/src/lib.rs:148` `SimulationObserver` with `on_step_start(pc, opcode)`,
  `on_step_end(cycles, &registers)`, `on_memory_write`, `on_peripheral_tick`. All methods
  default to empty.
- `Machine.observers: Vec<Arc<dyn SimulationObserver>>` at `crates/core/src/lib.rs:677`,
  initialised empty at `:746`, threaded into the CPU at `Machine.step()` `:895` and the
  secondary core `:913`.
- Cortex-M fan-out, INSIDE `step_internal`:
  - `on_step_start` at `crates/core/src/cpu/cortex_m.rs:859`, guarded by
    `if !_observers.is_empty()` (`:859`) — this is the existing zero-cost-when-no-observer
    guard for the start hook.
  - `on_step_end` at `crates/core/src/cpu/cortex_m.rs:2785`, but the 17-word `registers`
    array it passes is built UNCONDITIONALLY at `:2779-2783` even when there is no observer.
    (Capture-cost note in §5.)
- RISC-V has the analogous fan-out in its `step_internal` (same trait, same call shape).

### 2.3 The batch fast-path (audited — events are NOT dropped)
- `step` is a thin wrapper over `step_internal` (`crates/core/src/cpu/cortex_m.rs:620-627`).
- `step_batch` at `:629`: when `!config.batch_mode_enabled` it falls back to a per-step loop
  (`:636-641`); otherwise it loops calling `step_internal` per instruction (`:663` for the
  `SystemBus` fast-path, `:685` for the generic path). The only early-exit is the
  non-sequential-PC break at `:666`/`:688` (`pc_diff != 2 && pc_diff != 4 -> break`), which
  ends the batch but does NOT skip the observer fan-out for instructions it did run —
  `on_step_start`/`on_step_end` fire inside every `step_internal`.
- **Therefore: a PC-coverage observer attached to `machine.observers` sees every executed
  instruction in both batch and single-step mode.** This is the load-bearing correction
  versus the stale draft (which claimed batch mode bypassed observers and proposed a
  batch-mode kill-switch for coverage runs). No kill-switch is needed; the batch path is
  observer-faithful by construction. The only real subtlety is the early-exit semantics:
  branch-edge inference (§4) needs the *next* PC, and the batch break at `:666` means the
  next instruction's `on_step_start` lands in the next `step_batch` call — fine, because the
  observer is stateful across calls (it remembers the previous PC).

### 2.4 DWARF symbolication already present
- `crates/loader/src/lib.rs:456` `SymbolProvider`, built with gimli + addr2line
  (`:474` `new`), carrying a `line_map: HashMap<(String,u32), u64>` (`:466`) built by walking
  the line program at `:487-540` (stores the first address for each file:line).
- `lookup(addr) -> Option<SourceLocation>` at `:559` (returns file/line/function;
  `SourceLocation` at `:436`).
- `location_to_pc_nearest` at `:594` (reverse: file:line -> address).
- The line program walk at `:520-530` is where we get the address->(file,line) rows; for
  coverage we need the FULL set of statement rows (every `is_stmt` row), not just the first
  address per file:line. This means a NEW accessor on `SymbolProvider` (§4.1), not reuse of
  `line_map` (which dedups).

### 2.5 The existing `coverage/` module is NOT PC coverage (verified)
- `crates/core/src/coverage/mod.rs` + `probe.rs`: SVD-driven, behavioral *register*
  coverage — "does the model implement each register the chip's SVD declares," judged by
  observed read/write behavior (`probe.rs:5-18`). `RegStatus::{Modelled, Unmodelled,
  Indeterminate}`.
- `crates/cli/src/commands/coverage.rs:11` `run_coverage` and the `labwired coverage`
  subcommand drive THIS register probe (takes `--svd`, writes a register matrix JSON).
- **Name-collision flag:** the CLI verb `coverage` is already taken by register-modeling
  coverage. Firmware PC coverage must NOT reuse it. Use a distinct surface — see §6 naming.

### 2.6 Shared trace-event source (cross-plan anchor)
- `crates/hw-trace/src/event.rs:6` `enum TraceEvent { Placeholder }` with the literal
  comment "Plan 1 carries only a placeholder enum. Populated fully in Plan 2." (`:3`).
  THIS plan is the one that populates it. Coverage, the GDB stub (plan #5), execution-trace,
  and fault-injection (plan #3) must all emit/consume this one enum.

### 2.7 Determinism harness precedent
- `crates/cli/tests/determinism.rs:22` `test_determinism_smoke` runs the same script 5x and
  SHA-256s output files (`sha256_file` at `:15`) to assert byte-stability. The RunManifest
  reproducibility test (§4.4) extends this exact pattern to the manifest digest.

## 3. Architecture overview

```
                    machine.observers : Vec<Arc<dyn SimulationObserver>>
                                 |  (fan-out at cortex_m.rs:859 / :2785, risc-v analog)
        +------------------------+-----------------------------+
   TraceObserver          CoverageObserver (NEW)          (plan #3 fault sink)
   (exists)                  |  bitset[pc] + opcode-class mask
                             v
                   CoverageData (block-hit map + branch-edge map)
                             |  (symbolicate AFTER run, not per-step)
                             v  SymbolProvider.statement_rows() / branch_sites()
                   CoverageReport --> coverage.info (LCOV)
                                  --> coverage.json
                                  --> (rolled into RunManifest §4.3)

   write_outputs (main.rs:1874) --> RunManifest (NEW) --> run-manifest.json
                                                      \- canonical digest (sha256, no wall-clock)
```

Design rule: **capture is cheap and symbol-free during the run; symbolication happens once
at the end.** The observer records raw PC hits and raw branch edges into compact structures;
DWARF mapping runs against `SymbolProvider` after the loop, so the line-program walk never
sits on the hot path.

## 4. Component design

### 4.1 CoverageObserver (capture) — `crates/core/src/coverage_pc.rs` (NEW module)

New file, separate from the SVD `coverage/` module to avoid confusing register coverage with
PC coverage. (Module name `coverage_pc` deliberately distinct; re-exported from
`crates/core/src/lib.rs`.)

State (behind one `Mutex`, matching `TraceObserver`'s pattern at `core/src/trace.rs:39`):
- `executed` PC-hit set over the loaded executable address range. Two representations,
  chosen at construction from the ELF's executable-segment span:
  - dense `bitset` (1 bit per 2-byte Thumb slot) when the span is small (a few MB of code) —
    O(1) set, zero alloc per step;
  - sparse `HashSet<u32>` fallback for pathological spans.
  Bit index = `(pc - text_base) >> 1` (Thumb half-word granularity; RISC-V uses `>>1` too
  since RVC is 2-byte and base instr 4-byte — indexing on 2-byte slots covers both).
- `branch_edges: HashMap<u32, BranchEdge>` keyed by the *source* PC of a control-flow
  instruction, where `BranchEdge { taken: bool, not_taken: bool, taken_target: Option<u32>,
  fallthrough: Option<u32> }`. Populated by comparing `prev_pc + insn_len` (fallthrough) to
  the actual next `on_step_start` PC.
- `opcode_class_mask: u64` — a bitmask over a small opcode-class enum (ALU, load, store,
  branch-cond, branch-uncond, call, ret, exception-entry, etc.) OR'd in per step. Cheap
  decode-class derived from the opcode already in hand at `on_step_start`. Gives a coarse
  "what kinds of instructions did this firmware exercise" signal — useful as a sanity line
  and for fault-injection targeting (plan #3).
- `prev_pc: Option<u32>`, `prev_len: u8` — to infer the edge on the next step.

Hook usage:
- `on_step_start(pc, opcode)`: set the PC bit; OR the opcode class into the mask; if
  `prev_pc` was a control-flow instruction, resolve the edge: if `pc == prev_pc + prev_len`
  -> mark `not_taken`/fallthrough on the prev edge, else -> mark `taken` + record
  `taken_target = pc`. Then stash `prev_pc/prev_len`. This is the ONLY place branch
  taken/not-taken is derived — purely from "where did execution actually go next," exactly as
  the task specifies (branch taken/not-taken from next PC).
- `on_step_end`: unused by coverage (avoids the 17-word register-copy dependency).
- No symbolication here — keep it integer-only on the hot path.

Public accessor `take_coverage() -> CoverageData` mirrors `TraceObserver::take_traces`
(`core/src/trace.rs:58`).

### 4.2 DWARF statement + branch mapping — extend `SymbolProvider`

Add to `crates/loader/src/lib.rs` (alongside `line_map` at `:466`):
- `statement_rows() -> Vec<StmtRow>` where `StmtRow { addr: u64, file: String, line: u32,
  is_stmt: bool, end_sequence: bool }` — the FULL line-program rows (not deduped like
  `line_map`). Built by the same gimli walk already at `:487-540`, but emitting every row
  with `is_stmt`. This is the statement universe: total statements = count of distinct
  `is_stmt` rows; covered = those whose `addr` falls in an executed half-word slot.
- `branch_sites() -> Vec<BranchSite>` — addresses DWARF flags as branch points where
  available (prologue_end / column changes), plus the decode-derived control-flow sites from
  the CoverageObserver's `branch_edges`. The branch universe is the union; a branch is
  covered when both edges were observed.

Mapping (post-run, in CLI): for each executed bit, `addr -> (file,line)` via the statement
rows; aggregate hit counts per `(file,line)`. For branches, join `branch_edges` (source PC)
against `branch_sites` and emit, per line, `BRDA` records (LCOV branch data) with
taken/not-taken counts.

MC-DC is explicitly DEFERRED (§7): we emit statement + branch only. MC-DC needs
sub-condition decomposition of `&&`/`||` operands, requiring either column-level DWARF
condition tracking or compiler instrumentation — out of scope here, but the branch-edge
structure is the substrate it will build on.

### 4.3 RunManifest schema — `crates/cli/src/manifest.rs` (NEW) or inline in main.rs

`#[derive(Serialize, Deserialize)] struct RunManifest`:

```
manifest_schema_version: String          // new const, e.g. "1.0"
engine_version: String                   // env!("CARGO_PKG_VERSION") of cli crate + git sha if available
firmware: { path, sha256 }               // sha256 reuses main.rs:1891-1893
chip_model: { id, version }              // chip descriptor id + a model-version stamp (see risk R4)
configs: [ { path, sha256 } ]            // system manifest YAML + test script YAML, each hashed
seed: u64                                // the run seed (see risk R3 — must be made explicit)
limits: TestLimits                       // reuse config::TestLimits
results: {                               // deterministic subset of TestResult
    status, stop_reason, stop_reason_details,
    steps_executed, cycles, instructions,
    assertions: [ {assertion, passed} ],
    cpu_state_digest: String             // sha256 of canonical CpuSnapshot, NOT the full snapshot
}
coverage_summary: {                      // rolled up from CoverageReport
    statements: {total, covered}, branches: {total, covered},
    opcode_classes: [..]
}
fault_injections: [ FaultRecord ]        // CONTRACT for plan #3 — see §8
digest: String                           // sha256 over canonical JSON of ALL fields ABOVE this one
```

**Canonicalization & bit-reproducibility:**
- Serialize every map with sorted keys (BTreeMap, not HashMap) and stable field order.
- EXCLUDE wall-clock entirely from the digested region: no `duration`, no timestamps, no
  `Instant` (the run already tracks `duration` separately as a `write_outputs` arg `:1888`
  — keep it OUT of the manifest's digested section; if a human-facing `generated_at` is
  wanted, put it OUTSIDE the digest as a sibling field the digest does not cover).
- Compute `digest` by canonical-JSON-serializing the struct WITH the `digest` field omitted
  (serde skip on a placeholder, or a digest-less mirror struct), hashing the bytes, then
  filling `digest`.
- Float-free: cycles/steps are integers; coverage is integer counts; no `f64` in the
  digested region (the JUnit's `time_secs` float at `:2266` stays in JUnit only, never the
  manifest).
- Signing: the manifest is designed so `digest` is the thing a buyer signs (detached
  signature over the digest, or over the canonical bytes). Signing itself is out of scope;
  the schema guarantees a stable thing to sign.

### 4.4 Reproducibility CI test — `crates/cli/tests/manifest_reproducible.rs` (NEW)

Extend the `determinism.rs:22` pattern: run the same script twice into two output dirs, parse
both `run-manifest.json`, assert `digest` is byte-identical AND stable across a
re-serialization round-trip. The two separate process invocations already differ in wall
time, proving the digest is wall-clock invariant (no foreground sleep needed — and it is
banned in this environment anyway). A second negative test: mutating one config byte must
change the digest.

## 5. Overhead budget and the capture-cost fix

- **Target:** < 5% wall-time regression with coverage on, ~0% with it off.
- Off: the `on_step_start` guard at `cortex_m.rs:859` already makes the start hook free when
  `observers.is_empty()`. The `on_step_end` register-array build at `:2779-2783` is the one
  unconditional cost; it predates this plan and runs even with no observer. Optional
  micro-opt (flag as a separate cleanup, not required here): gate the `registers` array build
  behind `!_observers.is_empty()` too. Coverage does not use `on_step_end`, so it adds
  nothing there.
- On: per step the observer does one bit-set + one mask-OR + one branch-edge check — all
  integer, no alloc, one mutex lock. The mutex is the main cost (matches `TraceObserver`). If
  profiling shows the lock dominates, switch the bitset to relaxed atomics and drop the lock
  for the hit-set, keeping the lock only for `branch_edges`. Defer until measured.
- Symbolication is O(line-program rows + executed slots) ONCE at end — never on the hot path.

## 6. CLI / runner integration

- New flag on `TestArgs`: `--coverage <DIR-or-file>` (and/or `--coverage-format
  lcov,json`). When set, `execute_test_loop` constructs a `CoverageObserver` from the loaded
  ELF's text span and pushes it onto `machine.observers` right next to the `TraceObserver`
  push at `main.rs:1523`.
- New flag `--run-manifest <PATH>` (default: write `run-manifest.json` into `--output-dir`
  when `--coverage` or an explicit `--run-manifest` is set).
- In `write_outputs` (`main.rs:1874`): after the existing files, if a coverage observer was
  attached, call `take_coverage()`, build the `SymbolProvider` from the firmware path,
  produce `CoverageReport`, write `coverage.info` (LCOV) + `coverage.json`. Then assemble and
  write the `RunManifest`, reusing the firmware hash computed at `:1891`.
- JUnit: add `<property>` rows for `statement_coverage` / `branch_coverage` percentages in
  `write_junit_xml` (`:2342` is where properties are emitted), so existing dashboards see
  coverage without a new file. Optionally a synthetic `<testcase name="coverage.branch">`
  that fails below a `--coverage-min` threshold (cert gating).
- **Naming:** do NOT touch the existing `labwired coverage` verb (register modeling,
  `commands/coverage.rs`). Firmware PC coverage is a *flag on `labwired test`/`run`*, not a
  new top-level verb. If a verb is ever wanted, name it `coverage-firmware` or fold under
  `report`. Document the distinction in `--help`.

## 7. Phases & files touched

**Phase A — Capture (core).**
- NEW `crates/core/src/coverage_pc.rs`: `CoverageObserver`, `CoverageData`, `BranchEdge`,
  opcode-class enum + classifier.
- EDIT `crates/core/src/lib.rs`: `mod coverage_pc; pub use ...`.
- Tests: unit test that a hand-rolled instruction stream sets the expected bits, edges, and
  opcode classes (no ELF needed).

**Phase B — DWARF mapping (loader).**
- EDIT `crates/loader/src/lib.rs`: add `statement_rows()` and `branch_sites()` to
  `SymbolProvider`; new `StmtRow`/`BranchSite` types.
- Tests: against a committed fixture ELF with known source, assert statement count and a
  known covered/uncovered line.

**Phase C — Report + manifest (cli).**
- NEW `crates/cli/src/manifest.rs` (RunManifest + canonicalization + digest).
- NEW LCOV writer (small, in cli — `TN/SF/DA/BRDA/end_of_record`).
- EDIT `crates/cli/src/main.rs`: `TestArgs` flags; observer push near `:1523`; report +
  manifest emission in `write_outputs` near `:1996`; JUnit coverage properties at `:2342`.
- NEW const `MANIFEST_SCHEMA_VERSION`.

**Phase D — Reproducibility + cert tests.**
- NEW `crates/cli/tests/manifest_reproducible.rs` (extends `determinism.rs`).
- NEW `crates/cli/tests/coverage_lcov.rs` (golden LCOV against a fixture firmware).

**Phase E — Cross-plan convergence.**
- EDIT `crates/hw-trace/src/event.rs`: replace `TraceEvent::Placeholder` with real variants
  (instruction-retired{pc,opcode}, branch-edge{src,target,taken}, mem-write,
  exception-entry, fault-injected). Coverage capture, exec-trace (plan #5), and fault records
  (plan #3) all read/write THIS enum. Single event source the three plans converge on.

## 8. Critical cross-plan contracts

1. **One event source.** `hw-trace::TraceEvent` is the canonical event vocabulary. Coverage,
   GDB-stub-driven runs (plan #5), and execution-trace all derive from it. Do not invent a
   parallel event type. Phase E owns the enum population; plans #3/#5 consume it.

2. **GDB stub must attach the observer.** Verified: `crates/gdbstub/src/lib.rs` has ZERO
   `observers.push` calls. The stub steps via `target.machine.step_single()` /
   `machine.run()` at `:325`/`:327`, which DO route through `Machine.step()` -> the observer
   fan-out. So the stub does not *bypass* the step path — it simply never attaches any
   observer. **Contract:** a coverage-under-debug run must push the `CoverageObserver` onto
   `machine.observers` before the stub loop starts, exactly as `execute_test_loop` does. A
   one-line attach in stub setup, NOT a step-path change. (Correction vs the stale draft,
   which claimed the stub bypassed observers in the step loop itself.)
   - The DAP adapter already demonstrates the pattern — it pushes a `MemoryTracker` observer
     at `crates/dap/src/adapter.rs:231/241/251`. The GDB stub should mirror that.

3. **Fault-injection evidence record (plan #3 contract).** Plan #3's `cpu.inject_fault`
   (trait hook at `crates/core/src/lib.rs:217`) must, when it fires, emit a `FaultRecord` into
   the run's evidence. The contract THIS plan defines:
   `FaultRecord { kind, target, at_step, at_pc, effect }`, serialized into the
   `RunManifest.fault_injections` vector (§4.3) and into the digested region (so injected
   runs are still bit-reproducible given the same seed). Plan #3 produces the records; this
   plan owns the schema slot and the digest inclusion. Fault records also surface as a
   `TraceEvent::FaultInjected` variant (contract #1) so the trace and the manifest agree.

## 9. Risks

- **R1 — DWARF line-table fidelity at -O2.** Optimized firmware folds/duplicates lines;
  some statements map to multiple address ranges, some to none. Mitigation: count an
  `is_stmt` row covered if ANY of its address ranges is hit; report "no-code" lines
  separately so the denominator is honest. Document this in the report header, not hide it.
- **R2 — Branch-edge inference vs IT blocks / tail-calls.** On Cortex-M, IT-predicated
  instructions and `pop {pc}` returns complicate "next PC = fallthrough?" logic. The
  `step_internal` IT handling (`cortex_m.rs:868`) means a predicated-false instruction still
  advances PC by its length — so it reads as fallthrough, correct for branch coverage but
  under-counts predication coverage. Acceptable for statement+branch; note it.
- **R3 — Seed must be explicit.** The manifest claims a `seed`, but the sim has no single
  surfaced RNG seed (determinism today comes from the absence of nondeterminism, per
  `determinism.rs`). Action: audit for nondeterministic sources (HashMap iteration order in
  outputs, time-based peripheral init); if none, record `seed: 0` + a `nondeterminism: none`
  assertion backed by the reproducibility test. If any is found, it must be seeded or removed
  before the cert claim is credible.
- **R4 — Chip-model version stamp.** The manifest needs `chip_model.version`, but
  `ChipDescriptor` (`config/src/lib.rs:116`) has no version field today. Action: hash the
  resolved chip descriptor (canonical YAML) as the model version, OR add an explicit version
  field. Hashing is lower-friction and consistent with the config-hash approach.
- **R5 — Name collision** (`labwired coverage` = register coverage). Mitigated by §6: PC
  coverage is a flag, never the `coverage` verb.
- **R6 — Batch early-exit + branch edges across calls.** The batch break at
  `cortex_m.rs:666` ends a batch on a control-flow change; the observer must hold `prev_pc`
  across `step_batch` calls (it does, being `Arc`-shared state) so the edge resolves on the
  next call's first `on_step_start`. Covered by design (§4.1); flagged so an implementer does
  not reset state per batch.

## 10. Deferred
- MC-DC decomposition (needs condition-level DWARF or instrumentation) — branch-edge
  structure is the substrate; not in this plan.
- Cryptographic signing of the manifest (we guarantee a stable signable digest; key
  management / signature format is separate).
- `on_step_end` register-array gating micro-opt — independent cleanup.

---

## Corrections vs the stale-branch draft
1. **Batch mode does NOT bypass observers.** Verified at `cortex_m.rs:629-695` + `:859`/`:2785`:
   `step_batch` calls `step_internal` per instruction and the observer fan-out is inside it.
   No batch-mode kill-switch is needed for coverage; the stale draft's central mechanism was
   wrong. The only batch subtlety is cross-call `prev_pc` state (R6).
2. **The GDB stub does not bypass the step path** — it routes through `Machine.step()` via
   `step_single`/`run` (`gdbstub/src/lib.rs:325-327`); it merely never attaches an observer
   (zero `observers.push`). The fix is a one-line attach, not a step-loop change.
3. **`hw-trace::TraceEvent` is a live placeholder** whose own comment names THIS plan as the
   one that populates it (`event.rs:3`) — confirmed present on current main.
4. **`labwired coverage` already exists** as register-modeling coverage
   (`commands/coverage.rs`, `core/src/coverage/`), so PC coverage must not reuse that verb —
   a collision the stale draft missed.
5. **`SymbolProvider.line_map` dedups to first-address-per-line** (`loader/src/lib.rs:524-528`),
   so statement coverage needs a new full-row accessor, not the existing map.
