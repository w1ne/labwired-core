# Coverage Scoreboard

Date: February 22, 2026  
Scope: Priority-1 Top-5 matrix baseline for deterministic smoke.

This document is the baseline scoreboard referenced by the Top-20 coverage plan.
Values should be updated by CI artifacts from the matrix smoke workflow.

## Baseline Snapshot

| Metric | Baseline Value | Notes |
|---|---|---|
| Top-5 targets defined | `5` | Defined in `docs/spec/TOP20_COVERAGE_MATRIX.md` |
| Top-5 with runnable smoke script | `4/5` | `stm32f401-nucleo` is still backlog |
| Top-5 with deterministic evidence artifacts in matrix CI | `0/5` | Matrix workflow introduced; first run pending |
| Top-5 with unsupported-instruction audit artifacts | `0/5` | Audit step is integrated in matrix workflow; first run pending |
| Top-5 with explicit known-limitations entries | `2/5` | `stm32f103-bluepill`, `stm32h563-nucleo` |

## Target-Level Baseline

| Target ID | Smoke Script | Build Path | Baseline State | Evidence Path (when available) |
|---|---|---|---|---|
| `stm32f103-bluepill` | `examples/tests/stm32f103_integrated_test.yaml` | `cargo build -p firmware --target thumbv7m-none-eabi` | `scheduled` | `core/out/coverage-matrix/stm32f103-bluepill/` |
| `stm32h563-nucleo` | `examples/nucleo-h563zi/uart-smoke.yaml` | `cargo build -p firmware-h563-demo --release --target thumbv7m-none-eabi` | `scheduled` | `core/out/coverage-matrix/stm32h563-nucleo/` |
| `stm32f401-nucleo` | `planned` | `planned` | `blocked` | `n/a` |
| `rp2040-pico` | `planned` | `planned` | `blocked` | `n/a` |
| `riscv-ci-fixture` | `examples/ci/riscv-uart-ok.yaml` | `cargo build -p riscv-ci-fixture --release --target riscv32i-unknown-none-elf` | `scheduled` | `core/out/coverage-matrix/riscv-ci-fixture/` |

## Update Protocol

1. Matrix CI uploads per-target artifacts under `out/coverage-matrix/<target-id>/`.
2. Record pass/fail plus stop reason and assertion summary.
3. Update this file each time top-5 status changes.
4. Do not mark `green` without deterministic re-run evidence.
