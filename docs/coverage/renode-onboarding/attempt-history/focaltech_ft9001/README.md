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
