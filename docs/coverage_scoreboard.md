# Coverage Scoreboard

Date: February 22, 2026  
Scope: Priority-1 Top-5 matrix baseline for deterministic smoke.

This document is the baseline scoreboard referenced by the Top-20 coverage plan.
Values should be updated by CI artifacts from the matrix smoke workflow.

## Automated Scoreboard Artifact

The matrix workflow now generates:

- `coverage-matrix-scoreboard/scoreboard.md`
- `coverage-matrix-scoreboard/scoreboard.json`

These artifacts are published on every matrix run and summarized in GitHub Actions step summary.

Current CI hard gate:
- Top-5 executable deterministic targets must maintain >=80% pass rate.
- Gate is enforced in `core-coverage-matrix-smoke.yml` scoreboard job.

Latest validated run:
- Date: `2026-02-22`
- Run: `https://github.com/w1ne/labwired-core/actions/runs/22281072679`
- Summary: `6/6 pass`, `0 fail`, `0 missing`
- Top-5 gate: `5/5 pass` (`100%`, threshold `>=80%`)

## Current Snapshot

| Metric | Baseline Value | Notes |
|---|---|---|
| Top-5 targets defined | `5` | Defined in `docs/spec/TOP20_COVERAGE_MATRIX.md` |
| Top-5 with runnable smoke script | `5/5` | `stm32f401-nucleo` smoke package added |
| Top-5 with deterministic evidence artifacts in matrix CI | `5/5` | Published in `coverage-matrix-scoreboard` artifact |
| Top-5 with unsupported-instruction audit artifacts | `5/5` | Published per-target under matrix artifacts |
| Top-5 with explicit known-limitations entries | `3/5` | `stm32f103-bluepill`, `stm32h563-nucleo`, `stm32f401-nucleo` |

## Target-Level Baseline

| Target ID | Smoke Script | Build Path | Baseline State | Evidence Path (when available) |
|---|---|---|---|---|
| `stm32f103-bluepill` | `examples/demo-blinky/io-smoke.yaml` | `cargo build -p demo-blinky --release --target thumbv7m-none-eabi` | `passing` | `out/coverage-matrix/coverage-matrix-stm32f103-bluepill/` |
| `stm32h563-nucleo` | `examples/nucleo-h563zi/uart-smoke.yaml` | `cargo build -p firmware-h563-demo --release --target thumbv7m-none-eabi` | `passing` | `out/coverage-matrix/coverage-matrix-stm32h563-nucleo/` |
| `stm32f401-nucleo` | `examples/nucleo-f401re/uart-smoke.yaml` | `cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi` | `passing` | `out/coverage-matrix/coverage-matrix-stm32f401-nucleo/` |
| `riscv-ci-fixture` | `examples/ci/riscv-uart-ok.yaml` | `cargo build -p riscv-ci-fixture --release --target riscv32i-unknown-none-elf` | `passing` | `out/coverage-matrix/coverage-matrix-riscv-ci-fixture/` |
| `demo-blinky-stm32f103` | `examples/demo-blinky/io-smoke.yaml` | `cargo build -p demo-blinky --release --target thumbv7m-none-eabi` | `passing` | `out/coverage-matrix/coverage-matrix-demo-blinky-stm32f103/` |

Additional matrix sentinel target:
- `ci-fixture-armv6m` (included in matrix run, excluded from Top-5 hard gate)

## Update Protocol

1. Matrix CI uploads per-target artifacts under `out/coverage-matrix/<target-id>/`.
2. Matrix CI aggregates and publishes scoreboard artifacts via `scripts/generate_coverage_matrix_scoreboard.py`.
3. Update this file when the Top-5 scope, thresholds, or status policy changes.
4. Do not mark `green` without deterministic re-run evidence.
