# Postmortems

This folder contains incident postmortems for reliability, validation, and simulation regressions.

## Purpose

1. Capture what failed and why.
2. Record concrete corrective actions.
3. Prevent recurrence through process and automation updates.

## Index

1. [2026-02-14: Unsupported Instruction Validation Gap](./2026-02-14-unsupported-instruction-validation-gap.md)
2. [2026-02-15: STM32H563 GPIO ODR Failure (io-smoke)](./2026-02-15-io-smoke-failure.md)
3. [2026-02-16: v0.12.0 Release Regressions & CI Bypass](./2026-02-16-ci-bypass-and-loader-regressions.md)
4. [2026-02-16: Prefix32 Performance Regression](./2026-02-16-prefix32-performance-regression.md)

## Format

Each postmortem should include:

1. Incident summary
2. Impact
3. Timeline (with exact dates/times)
4. Root cause
5. Corrective actions
6. Prevention gates (tests/CI/docs/process)
