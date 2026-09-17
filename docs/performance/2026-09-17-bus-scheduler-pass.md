# Bus / scheduler per-instruction pass — 2026-09-17

Working notes for `perf/bus-scheduler-pass`. Goal: cut host instructions per
simulated instruction with **zero** behaviour change, on the loop the browser
runs (`Machine::advance` → `Cpu::step_batch`, tick interval 512).

## How everything here was measured

* **Ir/step** — host instructions retired per simulated CPU step, the metric
  `scripts/perf/board_perf.py` gates on. Two callgrind runs per board-mode at
  200 000 and 1 200 000 steps; the per-step cost is the **slope** between them,
  so ELF load, YAML parse and simulator construction cancel out. Deterministic
  to well under 1 % for a fixed binary, and it transfers to the wasm build.
* **CLI** — `cargo build --release -p labwired-cli --features event-scheduler`
  (the browser feature set), rustc 1.95.0, measured through
  `scripts/perf/board_perf.py --cli <binary> --boards nrf52840,stm32f405,esp32c3`.
* **Profiles** — `valgrind --tool=callgrind --cache-sim=no --branch-sim=no`
  over a 1 200 000-step `--batched` run, `callgrind_annotate` for self cost.
  Symbol rows below are **self** Ir; a Rust generic inlined from `core` is
  attributed to its defining file with the host function named after the colon,
  so `core/src/ptr/non_null.rs:…highest_priority_pending` is that function's
  inlined iterator, not a separate cost.

Numbers below are from **one machine, one binary per row**; only before/after
pairs measured the same way are compared.

## Baseline (commit 122ce319c + this worktree, before any change)

| board | mode | Ir/step | steps/batch |
|---|---|---|---|
| nrf52840 | batch | **229.8** | 511.9 |
| nrf52840 | step | 1419.8 | — |
| stm32f405 | batch | **229.0** | 511.9 |
| stm32f405 | step | 1030.8 | — |
| esp32c3 | batch | **402.0** | 511.9 |

## Profile — nrf52840, `perf-spin-nrf`, `--batched`, 1.2 M steps

Total 566,865,481 Ir. Per-step cost by the slope is 229.8 × 1.2 M ≈ 276 M Ir;
the rest is one-off YAML/ELF setup (`unsafe_libyaml`, `malloc`) which the slope
removes. Top self-cost rows:

| Ir | % total | symbol |
|---|---|---|
| 125,313,716 | 22.11 % | `cortex_m.rs:CortexM::step_batch` |
| 45,600,000 | 8.04 % | `cortex_m.rs:CortexM::highest_priority_pending` |
| 44,836,211 | 7.91 % | `unsafe_libyaml::reader::yaml_parser_update_buffer` (setup) |
| 26,400,000 | 4.66 % | `core/ptr/non_null.rs:CortexM::highest_priority_pending` (its inlined iterator) |
| 20,480,487 | 3.61 % | `unsafe_libyaml::scanner::yaml_parser_fetch_plain_scalar` (setup) |
| 20,418,697 | 3.60 % | `libc:_int_free` (setup) |
| 19,183,592 | 3.38 % | `core/slice/iter/macros.rs:CortexM::step_batch` (the `pending_exceptions` scans) |
| 16,988,042 | 3.00 % | `unsafe_libyaml::scanner::yaml_parser_fetch_more_tokens` (setup) |
| 14,508,371 | 2.56 % | `libc:malloc` (setup) |
| 14,051,935 | 2.48 % | `unsafe_libyaml` macros (setup) |
| 14,042,295 | 2.48 % | `libc:_int_malloc` (setup) |
| 9,107,970 | 1.61 % | `libc:free` (setup) |
| 9,000,000 | 1.59 % | `bus/accessors.rs:SystemBus::write_u32` |
| 8,602,898 | 1.52 % | `libc:__memcpy_avx_unaligned_erms` (setup) |
| 8,414,064 | 1.48 % | `core/option.rs:CortexM::step_batch` |

**Finding 1.** `highest_priority_pending` costs 45.6 M + 26.4 M = **72.0 M Ir
for 1.2 M instructions — 60 Ir on every single instruction**, i.e. ~26 % of the
whole per-step cost. `CortexM::step_execute` calls it unconditionally at the top
of every instruction (`highest_priority_pending().unwrap_or(0)`) and only then
asks whether anything is pending at all. With no exception pending — the
overwhelmingly common case — the call walks a `[u64; 4]` through
`iter().enumerate()` and returns `None`. The same shape appears on the real
Zephyr firmware (`nrf52840-zephyr-l0-hello.elf`): 45.6 M + 26.4 M + 4.8 M Ir.

**Finding 2.** The `self.pending_exceptions.iter().any(|&w| w != 0)` scans in
`step_batch` cost 19.2 M Ir (16 Ir/instruction) as a slice iterator rather than
a four-way OR.

## Profile — esp32c3, `perf-spin-esp32c3`, `--batched`, 1.2 M steps

Total 851,024,508 Ir; per-step cost ≈ 402 × 1.2 M ≈ 482 M Ir.

| Ir | % total | symbol |
|---|---|---|
| 138,401,533 | 16.26 % | `riscv.rs:RiscV::step` |
| 58,670,593 | 6.89 % | `unsafe_libyaml::reader::yaml_parser_update_buffer` (setup) |
| 39,600,066 | 4.65 % | `core/num/uint_macros.rs:SystemBus::read_u32` (inlined `Memory::read_u32` bounds maths) |
| 36,000,061 | 4.23 % | `bus/accessors.rs:SystemBus::read_u32` |
| 28,919,544 | 3.40 % | `riscv.rs:RiscV::step_batch` |
| 27,599,933 | 3.24 % | `core/num/uint_macros.rs:RiscV::step` |
| 24,869,798 | 2.92 % | `libc:_int_free` (setup) |
| 24,263,173 | 2.85 % | `unsafe_libyaml::scanner::yaml_parser_fetch_plain_scalar` (setup) |
| 24,000,023 | 2.82 % | `bus/routing.rs:SystemBus::find_peripheral_index` |
| 21,600,036 | 2.54 % | `core/intrinsics/mod.rs:SystemBus::read_u32` |
| 21,408,568 | 2.52 % | `unsafe_libyaml::scanner::yaml_parser_fetch_more_tokens` (setup) |
| 19,200,000 | 2.26 % | `core/ptr/non_null.rs:RiscV::step` |
| 17,653,376 | 2.07 % | `libc:malloc` (setup) |
| 16,844,967 | 1.98 % | `libc:_int_malloc` (setup) |
| 15,995,254 | 1.88 % | `unsafe_libyaml` macros (setup) |

**Finding 3.** `RiscV::fetch_opcode_u32`'s local code window **never hits on the
esp32c3**: the callgraph shows `bus.read_u32(pc)` taken 1,200,000 times — once
per instruction — for **130.8 M Ir inclusive, 109 Ir per instruction, 27 % of
the per-step cost**. `refill_fetch_window` only fills from a `FlashXipPeripheral`
or from `extra_mem`; a plain `flash` region (what `configs/chips/esp32c3.yaml`
declares at 0x4200_0000) matches neither, so the refill fails, `fetch_len` stays
0, and the refill is re-attempted on the *next* instruction as well — including
its `find_peripheral_index` call, which is where the 24.0 M Ir
(20 Ir/instruction) in routing comes from.

**Finding 4.** Inside that `read_u32`, the address is served by `flash`, but
only after `extra_mem_word` has probed all **five** `memory_regions` the C3
declares (iram, drom, rtc_fast, rom, rom_data), none of which is anywhere near
0x4200_0000. That linear probe is the 39.6 M + 21.6 M + 13.2 M + 10.8 M Ir of
inlined bounds maths and slice iteration attributed to `read_u32`.

## Differential oracle used for "zero behaviour change"

`--gpio-trace` writes nothing on these boards (no `--system` board_io wiring), so
the byte-identical gate uses three stronger artefacts per firmware, over seven
runs (nrf52840 Zephyr step + batch, nrf52840 tier1, stm32f405 tier1 step + batch,
esp32c3 tier1, esp32c3 demo), all at `--max-steps 5000000`:

* **stdout** — firmware console / TIER1 protocol lines;
* **`--bus-trace-out` JSON** — every UART/I²C/SPI event with its exact
  `cycle` stamp, so a change in *when* anything happens shows up;
* the **`[batched]` summary** — `instructions`, `batches`, `steps_per_batch`,
  `tick_interval`, `peripheral_ticks`, which pins the scheduling path itself.

21 sha256 hashes in total; all 21 must be unchanged.

## Result

Three commits, each measured on its own binary, all byte-identical to the
pre-series baseline on every one of the 21 hashes.

| board | mode | baseline | after (1) cortex-m | after (2) extra_mem | after (3) riscv fetch | total |
|---|---|---|---|---|---|---|
| nrf52840 | **batch** | 229.8 | 158.4 | 158.0 | 158.6 | **−31.0 %** |
| nrf52840 | step | 1419.8 | 1353.0 | 1354.0 | 1354.0 | −4.6 % |
| stm32f405 | **batch** | 229.0 | 157.5 | 157.5 | 157.5 | **−31.2 %** |
| stm32f405 | step | 1030.8 | 962.0 | 963.0 | 963.0 | −6.6 % |
| esp32c3 | **batch** | 402.0 | 402.0 | 358.0 | 262.0 | **−34.8 %** |

`batch` is the loop the browser runs (`Sim::step_batch` → `Machine::advance`),
so that column is the one that transfers to the wasm build. Its run-to-run
noise floor is ~±0.5 %, which is why the ±0.3–0.4 % moves in the columns where
a change does not apply are read as noise and not as findings.

`step` moves much less than `batch` because it is dominated by the per-instruction
machine boundary (`commit_advance_boundary` + a peripheral tick at interval 1),
which none of these three changes touches.

The `step` rows also sit above `scripts/perf/baselines.json` (nrf52840 1373.8,
stm32f405 949.8): that gap is present on the unmodified worktree too — it is
compiler/host drift against numbers recorded on CI, not something this branch
introduced, and it is why every comparison here is against the locally measured
baseline rather than against the checked-in file.

### What is left on the table

* `CortexM::step_batch` is still 22 % of the spin-fixture profile and is now,
  by a wide margin, the largest single item on the ARM path. That is the
  interpreter itself (decode-cache lookup, the `Instruction` match, register
  file), not orchestration.
* On the C3, `RiscV::step` is 16 % and the remaining `read_u32` per fetch is
  still a full bus walk — the fetch window cannot serve a plain `flash`
  region at all. Teaching `refill_fetch_window` about `SystemBus::flash`
  would remove that walk outright, but it would also stop the fetch
  incrementing `memory_reads`, which `bus.access_counts()` reports; that is a
  visible change and therefore out of scope for a pass whose contract is zero
  behaviour change.
* Per-tick (so amortised 512×, not a lever here):
  `collect_enabled_nvic_interrupts` does 16 `SeqCst` atomic loads per tick.

## Re-measured on current main after merging it in

The branch was merged with `origin/main` at `80c01390d` and everything above
re-run against a binary built from that exact commit, so the PR's numbers are
against the main it will merge into rather than against its branch point:

| board | mode | main 80c01390d | branch 36bc188e6 | delta |
|---|---|---|---|---|
| nrf52840 | **batch** | 230.0 | 158.6 | **−31.0 %** |
| nrf52840 | step | 1365.1 | 1295.4 | −5.1 % |
| stm32f405 | **batch** | 229.2 | 157.7 | **−31.2 %** |
| stm32f405 | step | 973.1 | 903.4 | −7.2 % |
| esp32c3 | **batch** | 402.2 | 262.2 | **−34.8 %** |

The differential harness was re-captured on `80c01390d` as well (the
`[batched]` summary line changed in #1145, so the older hashes no longer
apply) and all 21 hashes match between that binary and the branch.
