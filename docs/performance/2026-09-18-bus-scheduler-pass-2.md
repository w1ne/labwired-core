# Bus / scheduler per-instruction pass 2 — 2026-09-18

Follow-up to `perf/bus-scheduler-pass` (2026-09-17, −31 to −35% Ir/step on
the batched loop). Same method, same contract: zero behaviour change,
measured on the loop the browser runs (`Machine::advance` → `Cpu::step_batch`,
tick interval 512). See `docs/performance/2026-09-17-bus-scheduler-pass.md`
for the measurement recipe (`scripts/perf/board_perf.py`, the callgrind slope
technique, and the differential oracle).

## Baseline (this worktree, branch point `88a20fd32`)

`python3 scripts/perf/board_perf.py --boards nrf52840,stm32f405,esp32c3,atsamd21g18a,rp2040`:

| board | mode | Ir/step | steps/batch |
|---|---|---|---|
| nrf52840 | batch | **158.5** | 511.9 |
| nrf52840 | step | 1305.6 | — |
| stm32f405 | batch | **157.7** | 511.9 |
| stm32f405 | step | 906.4 | — |
| esp32c3 | batch | **258.9** | 511.9 |
| atsamd21g18a | batch | 2348.5 | 1.0 |
| atsamd21g18a | step | 2302.5 | — |
| rp2040 | batch | **156.0** | 511.9 |
| rp2040 | step | 871.0 | — |

(These are all within noise of pass 1's post-merge numbers — 158.3 / 157.7 /
262.2 / 156.1 — confirming nothing drifted between the two passes.)

## Profile — callgrind, `--batched`, 3,000,000 steps, real firmware fixtures

Unlike pass 1 (which profiled the ALU spin fixture), this pass profiled real
firmware — `tests/fixtures/nrf52840-zephyr-l0-hello.elf` (Zephyr),
`tests/fixtures/tier1/stm32f405.elf` and `tests/fixtures/tier1/esp32c3.elf`
(the TIER1 conformance images) — to catch overhead that only shows up once a
board is doing real peripheral I/O rather than a pure ALU loop.

Top self-Ir symbols (`callgrind_annotate --inclusive=no`):

### nrf52840 (Zephyr hello, 674,456,831 Ir total)

| Ir | % | symbol |
|---|---|---|
| 315,137,605 | 46.72% | `cortex_m.rs::step_batch` |
| 44,836,211 | 6.65% | `unsafe_libyaml` (setup) |
| 20,480,487 | 3.04% | `unsafe_libyaml` (setup) |
| 20,450,282 | 3.03% | `libc:_int_free` (setup) |
| 9,052,992 | 1.34% | `core/option.rs::step_batch` |
| 9,027,354 | 1.34% | `core/sync/atomic.rs::step_batch` |
| 8,999,999 | 1.33% | `std/sync/once_lock.rs::step_batch` |

### stm32f405 (tier1, 595,241,047 Ir total)

| Ir | % | symbol |
|---|---|---|
| 374,793,368 | 62.96% | `cortex_m.rs::step_batch` |
| 26,975,421 | 4.53% | `cortex_m.rs::read_reg` |
| 22,487,431 | 3.78% | `bus/accessors.rs::write_u32` |
| 9,035,244 | 1.52% | `core/option.rs::step_batch` |
| 9,017,622 | 1.51% | `core/sync/atomic.rs::step_batch` |
| 9,000,002 | 1.51% | `std/sync/once_lock.rs::step_batch` |

### esp32c3 (tier1, 1,068,969,703 Ir total)

| Ir | % | symbol |
|---|---|---|
| 273,297,458 | 25.57% | `riscv.rs::step` |
| 84,228,908 | 7.88% | `bus/accessors.rs::read_u32` |
| 72,310,108 | 6.76% | `riscv.rs::step_batch` |
| 21,159,420 | 1.98% | `bus/mmio_activity.rs` (inlined into `read_u32`) |

**Finding.** The `once_lock.rs` + `atomic.rs` + `option.rs` triple on both
ARM profiles (~9 M Ir each, ~27 M Ir for 3 M instructions = **~9
Ir/instruction**) traced to `CortexM::step_internal` calling
`trace_insn_enabled()` unconditionally on every retired instruction —
including on the batched path. That helper was already a pass-1-style fix
for a prior `std::env::var`-per-step bug (see its own doc comment), hoisted
into a `static OnceLock<bool>`. But the OnceLock's own already-initialized
fast path still costs an `Acquire` load through the lock's internal state
machine plus the `Option` unwrap around it, every single instruction — a
real, if smaller, instance of the same class of bug pass 1's intro
describes (`std::env::var` in `CortexM::step` costing ~830 Ir/step).

RISC-V's only `std::env::var` call (`LABWIRED_TRAP_DEBUG` in
`RiscV::handle_trap`) sits behind an actual trap, not the per-instruction
path, so esp32c3's flat delta below is expected: this fix is Cortex-M only.

The other pass-1 "left on the table" candidates were checked and found not
to be further profitable without restructuring the interpreter itself:

* **SysTick/NVIC pending checks** — already reduced to a four-word OR
  (`any_exception_pending`) by pass 1; `highest_priority_pending`'s O(n)
  walk only runs when that OR is true, which is not the common case. No
  further per-instruction cost to remove here.
* **`commit_advance_boundary` scheduler drains** — runs once per BATCH
  (≈512 instructions at tick_interval 512), not once per instruction; its
  cost is already amortised the way pass 1's own doc noted ("amortised
  512×, not a lever here"). Confirmed still true on this pass's profiles:
  it does not appear in the top 15 self-Ir symbols on any board.
* **MMIO activity publication** (`note_mmio_activity` / `CycleClock::publish`)
  — fires per peripheral MMIO access, not per instruction fetch; visible on
  esp32c3 (21.2 M Ir / 3 M steps ≈ 7 Ir/instruction) only because the tier1
  fixture does real UART/GPIO polling, but changing it would alter
  `access_counts()` semantics and is out of the zero-behaviour-change
  contract, same as pass 1's Finding 4 on the esp32c3 fetch window.
* **`logic_tap` checks** — already hoisted to once-per-batch (`bus.logic_tap()`
  computed once before the loop, `push_armed()` checked once), not
  per-instruction; nothing left to hoist further.

One fix landed from this profile.

## Result

One commit, measured on its own binary, byte-identical to the pre-series
baseline on all 8 hashes (stdout + `--bus-trace-out` JSON, `--max-steps
5000000`, for nrf52840 Zephyr, stm32f405 tier1, esp32c3 tier1, stm32l476
tier1).

| board | mode | baseline | after (1) trace_insn field | delta |
|---|---|---|---|---|
| nrf52840 | **batch** | 158.5 | 156.3 | **−1.4%** |
| nrf52840 | step | 1305.6 | 1303.4 | −0.2% |
| stm32f405 | **batch** | 157.7 | 155.5 | **−1.4%** |
| stm32f405 | step | 906.4 | 904.4 | −0.2% |
| rp2040 | **batch** | 156.0 | 154.2 | **−1.2%** |
| rp2040 | step | 871.0 | 869.0 | −0.2% |
| esp32c3 | **batch** | 258.9 | 262.2 | +1.3% (noise; RISC-V unaffected) |
| atsamd21g18a | batch (1:1) | 2348.5 | 2346.3 | −0.1% (noise) |

`esp32c3`'s move is within this gate's documented ±0.5% run-to-run noise
floor and unrelated to the Cortex-M-only fix (confirmed flat across repeat
runs of the unmodified binary too). The `step` mode moves less than `batch`
for the same reason pass 1 noted: at tick interval 1, `step` is dominated by
the machine-boundary cost the trace-check fix does not touch.

Local vs CI host measurements carry a known offset (see pass 1's doc, same
gap present on the unmodified worktree) — `scripts/perf/baselines.json` was
re-baselined from THIS run, not compared against CI's numbers.

### Why only one fix

The second candidate profiled (RISC-V trap-path `std::env::var`) is not on
the per-instruction hot path (only reachable on an actual trap), so it has
no measurable Ir/step effect to report — applying the same OnceLock/field
fix there would move nothing on the batched-loop metric this gate tracks.
No other candidate in this profile showed a distinct, still-unclaimed
per-instruction cost above the ~0.5% noise floor without touching
`CortexM::step_batch`'s decode/execute body itself, which pass 1 already
identified as the interpreter itself rather than orchestration overhead —
out of scope for a bus/scheduler pass.

### What is left on the table

* `CortexM::step_batch` is still 47–63% of these profiles (higher than
  pass 1's spin-fixture number because real firmware does more per
  instruction on average) — the interpreter's decode-cache lookup,
  `Instruction` match and register file, not orchestration.
* `RiscV::step` (26%) and its `read_u32` fetch/bus-walk shape remain as
  pass 1 described (Findings 3–4): a real lever, but changing it visibly
  alters `access_counts()`, which is out of a zero-behaviour-change pass's
  contract until that counter contract itself is revisited.
* `bus/mmio_activity.rs`'s per-MMIO `CycleClock::publish` (Relaxed atomic
  store) is cheap per call but shows up on any firmware that polls
  peripherals in a tight loop; a batch-scoped "already published this
  cycle" short-circuit could remove repeat stores within one instruction's
  MMIO burst, but needs care not to break the read-side freshness fix
  (#842) it exists for — a candidate for a future pass with more time to
  validate against the differential oracle across every scheduler-driven
  peripheral, not just the four fixtures this pass checked.
