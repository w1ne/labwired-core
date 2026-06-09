# LabWired Firmware Fuzzing — scope

**Status:** scoping · **Date:** 2026-06-09

## Why (the wedge)

LabWired is the **rock-solid firmware platform** — silicon-validated, deterministic.
That fidelity is a *vitamin* for a human dev (they flash the board to check) and a
*painkiller* the moment the loop is **automated**, because no human is there to
catch the sim lying. **Fuzzing is the first automated loop where rock-solid is
mandatory and there's budget** (EU CRA, FDA premarket cyber, UNECE R155 all force
connected-device firmware testing). It doesn't make us a security company — it
**proves** the platform thesis in the workflow where it matters most.

The defensible edge: emulation-based firmware fuzzers (Unicorn-AFL, Fuzzware,
P2IM, Qiling) drown in **false positives** — crashes the real chip wouldn't have,
because the peripheral model is wrong. We have the two things that kill that:
1. a sim **validated against silicon** (the oracle/conformance/HIL corpus), and
2. a **HIL bench** to *auto-confirm* every crash on the real part.

**Positioning:** *coverage-guided firmware fuzzing with silicon-confirmed crashes
— zero false positives.* "Fuzz at scale in sim, confirm on real silicon."

## Architecture

LabWired-as-fuzz-target, the proven Unicorn-AFL shape but silicon-faithful:

```
AFL++ (mutation + corpus + scheduling)
   │ input bytes
   ▼
LabWired run  ──emits──▶ edge-coverage bitmap (AFL shared-mem)
   │ fuzz input injected at a peripheral input surface (UART RX first)
   │ crash oracle: HardFault/Bus/Usage/MemManage, lockup, watchdog, hang(timeout)
   │ fast reset per iteration (post-boot snapshot restore)
   ▼
crash? ──▶ HIL confirm: flash F103, replay input over real UART, observe fault
            ──▶ CONFIRMED (real bug) | SIM-ONLY (false positive, model gap to fix)
```

We **reuse** AFL++ for mutation (don't build a fuzzer) and feed it firmware
coverage, exactly like Unicorn-AFL / Fuzzware.

## Reused vs new

| Reused (already built) | New (this work) |
|---|---|
| deterministic sim, `Machine`, ELF load | edge-coverage bitmap emission (AFL-style) |
| fault detection, lockup observable | fuzz-input injection surface (UART RX → byte stream) |
| `runtime_snapshot` (fast reset) | crash oracle (faults/lockup/watchdog/timeout → verdict) |
| openocd / HIL (`hil/`, oracle harness) | AFL++ harness (forkserver/persistent + shmem map) |
| UART model, CLI, MCP | HIL crash-confirm + triage (the wedge) |
| silicon-validation corpus | `labwired fuzz` CLI / MCP tool / CI action |

## Phases

### Phase 0 — Target + oracle (days) — ✅ DONE 2026-06-09
- Target firmware: a **UART command/protocol parser** on F103 (clean external-input
  surface, plant one known bug for the demo). Board already on the bench.
- Crash oracle: HardFault/BusFault/UsageFault/MemManage, core lockup, IWDG/WWDG
  reset, and instruction-count **timeout** (hang). Define the verdict enum.
- Injection point: firmware UART `DR` reads return successive fuzz bytes; stream
  end → terminate run.
- **Exit:** agreed target + crash definition + injection contract.
- **Done:** `firmware-f103-fuzztarget` (parser w/ planted overflow) + `f103_fuzz_phase0.rs`
  prove clean→DONE / overflow→FAULT in sim AND on the bench F103 (same crash input
  reproduces on real silicon). Injection = RAM buffer (openocd-replayable), not UART.
  **Finding:** the sim surfaces a CPU fault as a step `Err` rather than vectoring to
  the HardFault handler like silicon — Phase-1 fidelity item (recovering handlers
  would diverge).

### Phase 1 — Sim fuzzing primitives (≈1 wk) — ✅ DONE 2026-06-09
- **Coverage:** AFL edge map (`map[(prev_pc>>1) ^ cur_pc]++`) emitted from the CPU
  step loop, behind a `fuzz` feature so normal/JIT runs are unaffected.
- **Input injection:** a fuzz source feeding the UART RX model from a `&[u8]`.
- **Crash detection:** surface the oracle as a `FuzzResult { coverage, verdict }`.
- **Fast reset:** snapshot the post-boot machine once; restore per iteration.
- **Exit:** `labwired fuzz-run <fw> <input>` → (coverage, verdict), deterministic,
  **≥1k execs/sec** on the interpreter. (Interpreter only — coverage fights the JIT.)
- **Done:** `crates/labwired-fuzz` — edge coverage read from the CPU PC each step
  (no core changes), `Target::run` → `(CovMap, Verdict)`, + a minimal coverage-
  guided loop. It **finds the planted overflow in 239 iterations** from a benign
  seed (`finds_planted_bug` test). Fast-reset = fresh machine per run (snapshot is
  a perf follow-up). **Insight for Phase 3:** the crash over-reads RAM past the
  input, so the harness must zero the input region on sim AND silicon for a crash
  to reproduce identically. Phase 2 swaps the toy loop for AFL++/LibAFL.

### Phase 2 — Real fuzzer integration (≈1 wk) — ✅ DONE 2026-06-09
- Drive with a production fuzzing engine, reusing its mutation engine + scheduler.
- Seed corpus + crash dir + dedup by coverage.
- **Exit:** the real engine driving LabWired **finds the planted bug**; coverage climbs.
- **Done** (core: `labwired-fuzz` `libafl` feature; CLI `fuzz-libafl` feature):
  chose **LibAFL** over AFL++ — our engine is already Rust and `Target::run` is a
  natural in-process executor, so no forkserver/shmem glue. `src/libafl_engine.rs`
  wraps the sim as a LibAFL `InProcessExecutor`: `StdMapObserver` over the AFL edge
  bitmap, `MaxMapFeedback` + `QueueScheduler`, `havoc_mutations` via
  `HavocScheduledMutator`, `CrashFeedback` → solutions corpus. `fuzz()` /
  `fuzz_collect()` delegate to it when the feature is on; the built-in loop stays
  the dependency-free default (keeps CI + the HIL test light). Both find the
  planted overflow; LibAFL explores a richer space (discovers multi-frame inputs
  the built-in mutator never produces). Feature-gated because LibAFL is a large
  dependency tree — `cargo build -p labwired-cli --features fuzz-libafl`.

### Phase 3 — HIL crash-confirm (the wedge) (≈1 wk) — ✅ DONE 2026-06-09
- For each unique sim crash: flash F103, replay the input over real UART, detect a
  fault on silicon (SWD fault status / watchdog reset / no-response).
- Classify **CONFIRMED** (real, exploitable on hardware) vs **SIM-ONLY** (false
  positive → a model gap to feed back into the validation corpus).
- **Exit:** a triage report (sim crashes, silicon-confirmed subset, FP rate) on the
  bench F103. This is the demo that no competitor can run.
- **Done** (core #210): `labwired-fuzz::fuzz_collect()` gathers N distinct crashes;
  `hw-oracle/tests/f103_fuzz_hil_confirm.rs` fuzzes in sim → flashes once → replays
  each crash on the F103 (input region zeroed both sides → over-read crash is
  deterministic) → classifies CONFIRMED vs SIM-ONLY + FP rate. Replay = SWD
  RAM-inject (no UART adapter wired). Silicon-clean = reaches DONE; fault marker or
  hang = confirmed. **Bench result: 8 distinct sim crashes, 8/8 confirmed on
  silicon, 0% false positives.** (The planted firmware has one real bug, so 0% FP
  is correct; the classifier exercises the full silicon-confirm path either way.)

### Phase 4 — Package + story (days) — ✅ DONE 2026-06-09
- `labwired fuzz` CLI subcommand + MCP tool (`fuzz_firmware`) + a CI-action variant.
- Demo + write-up: "coverage-guided firmware fuzzing, silicon-confirmed, zero false
  positives" — contrasted with Unicorn-AFL/Fuzzware FP pain.
- **Exit:** a runnable `labwired fuzz` and a reproducible demo.
- **Done** (core: `labwired fuzz` subcommand; MCP `labwired_fuzz` tool v0.5.0):
  - `labwired fuzz --chip --system --firmware [--seed-input H] [--collect N]
    [--crashes-out F] [contract addr flags]` — runs the coverage-guided engine,
    writes crashing inputs as JSON, **exits non-zero on a crash (CI gate)**.
    Contract addresses/markers default to the F103 fuzz target, all overridable.
  - `@labwired/mcp` `labwired_fuzz` tool (board_id + ELF → distinct crashes,
    hex + raw bytes; isError=false since a found crash is a finding, not a tool
    error). README + version bumped to v0.5.0.
  - Write-up: `docs/guides/firmware-fuzzing.md` (contract table, CLI usage,
    HIL-confirm, copy-paste GitHub Actions gate, MCP usage).
  - Input surface = RAM buffer (SWD-replayable, no UART adapter). UART/DMA input
    surfaces are the post-thesis generalization (see Risks).

## Risks / unknowns

- **Input-surface modeling is the hard part** (it's literally what Fuzzware/P2IM
  research is about): real firmware takes input via DMA, interrupts, multiple
  peripherals. Start **UART-only**; generalize (SPI/I2C/network) after the thesis
  is proven. Don't boil the ocean in v1.
- **Coverage throughput vs JIT** — coverage instrumentation defeats the JIT; run
  fuzzing on the interpreter. Need ≥1k execs/sec or the loop is too slow; if short,
  optimize reset (snapshot) before mutation volume.
- **HIL-confirm is serial (1 board)** — fine: sim finds crashes at scale, HIL
  confirms the *unique* few. Throughput mismatch is expected and acceptable.
- **The "zero false positives" claim is load-bearing** — it's only true because the
  HIL-confirm gates it. Always report sim-found vs silicon-confirmed separately;
  never claim a crash is real until silicon says so.

## Definition of done (v1)
A `labwired fuzz` run on the F103 UART target that: finds the planted bug
coverage-guided, auto-replays each crash on real silicon, and emits a triage
report separating silicon-confirmed bugs from sim-only false positives — the
end-to-end "fuzz in sim, confirm on silicon" loop, on one chip, one input surface.
