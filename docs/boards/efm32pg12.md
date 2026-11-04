# EFM32PG12 Renode onboarding smoke

**Architecture:** ARM Cortex-M4F.

## Imported execution profile

`configs/chips/onboarding/efm32pg12.yaml` retains the imported 1 MiB flash and
256 KiB RAM map and exposes the model-backed USART0 at `0x4000c000`. The other
generated peripheral windows were omitted because this smoke does not exercise
them.

## Current evidence

The exact imported-system smoke in
`examples/renode-onboarding/efm32pg12-smoke.yaml` executed 100,000 instructions.
It checked that USART0 emitted `EFM32PG12 USART0 smoke` and that execution
reached the configured `max_steps` limit; both assertions passed.

Build and simulator logs, UART output, JUnit assertions, result JSON, and a
snapshot are recorded under
[`docs/coverage/renode-onboarding/metrics/efm32pg12/`](../coverage/renode-onboarding/metrics/efm32pg12/).
The initial malformed test-script attempt, which executed no instructions, is
preserved under
[`docs/coverage/renode-onboarding/attempt-history/efm32pg12/`](../coverage/renode-onboarding/attempt-history/efm32pg12/).
The full imported inventory is summarized in the
[`Renode onboarding scoreboard`](../coverage/renode-onboarding-scoreboard.md).

This is repository CLI smoke evidence for the imported Cortex-M4F target's
memory map and USART0 path. The fixture does not execute floating-point
instructions. It does not establish full EFM32PG12 peripheral coverage, hosted
`labwired_verify` model verification, or hardware confirmation.
