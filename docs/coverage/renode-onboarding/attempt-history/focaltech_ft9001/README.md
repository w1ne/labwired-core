# FocalTech FT9001 smoke attempt history

## Attempt 01

The first run failed at `run_smoke` with `config_error` before executing any
instruction: the smoke script declared `schema_version: "1.0"`, but
`uart_injections` require `"1.2"`. The CLI and firmware built. The metrics and
logs are retained under `attempt-01/`.

The script was bumped to `"1.2"` and rerun.

## Attempt 02

The platform booted, stopped at `max_steps`, and printed every expected UART
line. The `uart_ordered` and stop-reason assertions passed. The first
assertion failed because it used `\A`, which LabWired's `uart_regex` engine
(`crates/cli/src/regex.rs`) does not support. That engine anchors with `^`.
Retained under `attempt-02/`.

The pattern was changed to `^FT9001 SMOKE\r\n` and rerun; the current result
is under `../../metrics/focaltech_ft9001/`.

## Pre-IRQ-removal pass

Before narrowing the imported descriptor to drop USART2's unmodelled IRQ 33,
the passing metrics, logs and simulator artifacts were archived under
`2026-11-06-pre-irq-removal/`. That run executed 4,000,000 instructions and
passed all three smoke assertions. The current post-change evidence remains
under `../../metrics/focaltech_ft9001/`.

## CPM, reset and TRNG models (2026-11-15)

The passing USART2-only metrics before this change are archived under
`2026-11-15-before-sys-blocks/`.

The first exact-target run with the new descriptor failed at `run_smoke` with
`Failed to read firmware ... No such file or directory`: the inventory built
the firmware into the shell's external `CARGO_TARGET_DIR`, but the smoke
script loads it from the worktree `target/`. No instruction ran. Retained
under `2026-11-15-missing-firmware-external-target-dir/`. The rerun used
`CARGO_TARGET_DIR=$(readlink -f target)` and passed 3/3; the current result
is under `../../metrics/focaltech_ft9001/`.

`2026-11-15-negative-control-head-descriptor/` runs the same firmware on the
HEAD descriptor: FAIL 1/3, precise BusFault at 0x40002007 (RSR).
