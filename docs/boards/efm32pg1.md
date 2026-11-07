# EFM32PG1 Renode onboarding smoke

**Architecture:** ARM Cortex-M4F.

## Imported execution profile

`configs/chips/onboarding/efm32pg1.yaml` keeps the imported 256 KiB flash and
32 KiB RAM map and exposes the modeled USART0 at `0x4000c000`. Other generated
peripheral windows are omitted because this smoke does not exercise them.

## Current evidence

The exact imported-system smoke in
`examples/renode-onboarding/efm32pg1-smoke.yaml` executed 100,000 instructions.
USART0 emitted `EFM32PG1 USART0 smoke` and execution reached the configured
`max_steps` limit; both assertions passed.

Build and simulator logs, UART output, JUnit assertions, result JSON, and a
snapshot are recorded under
[`docs/coverage/renode-onboarding/metrics/efm32pg1/`](../coverage/renode-onboarding/metrics/efm32pg1/).
The initial wrong-address firmware attempt, which faulted at unmapped MMIO and
emitted no UART bytes, is preserved under
[`docs/coverage/renode-onboarding/attempt-history/efm32pg1/`](../coverage/renode-onboarding/attempt-history/efm32pg1/).
The full imported inventory is summarized in the
[`Renode onboarding scoreboard`](../coverage/renode-onboarding-scoreboard.md).

This is repository CLI smoke evidence for the imported Cortex-M4F target's
memory map and USART0 path. It does not establish full EFM32PG1 peripheral
coverage, hosted `labwired_verify` model verification, or hardware confirmation.
