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

## Baseline Snapshot

| Metric | Baseline Value | Notes |
|---|---|---|
| Top-5 targets defined | `5` | Defined in `docs/spec/TOP20_COVERAGE_MATRIX.md` |
| Top-5 with runnable smoke script | `5/5` | `stm32f401-nucleo` smoke package added |
| Top-5 with deterministic evidence artifacts in matrix CI | `0/5` | Matrix workflow introduced; first run pending |
| Top-5 with unsupported-instruction audit artifacts | `0/5` | Audit step is integrated in matrix workflow; first run pending |
| Top-5 with explicit known-limitations entries | `3/5` | `stm32f103-bluepill`, `stm32h563-nucleo`, `stm32f401-nucleo` |

## Target-Level Baseline

| Target ID | Smoke Script | Build Path | Baseline State | Evidence Path (when available) |
|---|---|---|---|---|
| `stm32f103-bluepill` | `examples/tests/stm32f103_integrated_test.yaml` | `cargo build -p firmware --target thumbv7m-none-eabi` | `scheduled` | `core/out/coverage-matrix/stm32f103-bluepill/` |
| `stm32h563-nucleo` | `examples/nucleo-h563zi/uart-smoke.yaml` | `cargo build -p firmware-h563-demo --release --target thumbv7m-none-eabi` | `scheduled` | `core/out/coverage-matrix/stm32h563-nucleo/` |
| `stm32f401-nucleo` | `examples/nucleo-f401re/uart-smoke.yaml` | `cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi` | `scheduled` | `core/out/coverage-matrix/stm32f401-nucleo/` |
| `rp2040-pico` | `planned` | `planned` | `blocked` | `n/a` |
| `riscv-ci-fixture` | `examples/ci/riscv-uart-ok.yaml` | `cargo build -p riscv-ci-fixture --release --target riscv32i-unknown-none-elf` | `scheduled` | `core/out/coverage-matrix/riscv-ci-fixture/` |

## Update Protocol

1. Matrix CI uploads per-target artifacts under `out/coverage-matrix/<target-id>/`.
2. Matrix CI aggregates and publishes scoreboard artifacts via `scripts/generate_coverage_matrix_scoreboard.py`.
3. Update this file when the Top-5 scope, thresholds, or status policy changes.
4. Do not mark `green` without deterministic re-run evidence.
