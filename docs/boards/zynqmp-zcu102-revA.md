# ZCU102 Evaluation Board (Rev A) imported profile (`zynqmp-zcu102-revA`)

## Measured profile

The imported Renode target is narrowed to one Cortex-A53 processing element on
the AArch64 backend, the imported low-DDR window (`0x00030000`, just under
2 GiB), and PS UART0 at `0xFF000000` using the generic UART engine with the
Cadence (`xuartps`) register layout: polled TX through `Channel_sts`/`TX_RX_FIFO`.
The other three A53s, the Cortex-R5F RPU, GIC-400, generic timers, UART1, TTCs,
GEMs, GPIO, I2C, IPI, RTC, PMU, QSPI and the board-level I2C devices from the
import are omitted; the import answered most of them with zero-reading stubs.

An earlier report on this page described a 2026-10-18 run of a Thumb
Cortex-M fixture against this descriptor. That run did not execute A53 code
and is superseded.

## Smoke evidence

The exact imported system ran the committed AArch64 fixture
(`tests/fixtures/zynqmp-a53-uart-smoke.elf`, built by
[`scripts/build_zynqmp_a53_uart_smoke.sh`](../../scripts/build_zynqmp_a53_uart_smoke.sh)).
It polls UART0 `Channel_sts.TXFULL`, writes `ZYNQMP A53 UART SMOKE` to the TX
FIFO, then spins in an integer loop. The smoke asserts the banner in the
captured UART output and the bounded `max_steps` stop after 100,000
instructions. With UART0 removed from the descriptor the same fixture faults
on its first status read, so the banner assertion depends on the model.

This does not test interrupts, baud timing, RX, the GIC, timers, multicore
boot, the FSBL/PMU flow, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/zynqmp-zcu102-revA/` contains the
per-target metric, JUnit result, simulator result, UART log, snapshot and run
log.
