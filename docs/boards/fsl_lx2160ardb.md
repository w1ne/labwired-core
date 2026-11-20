# NXP Layerscape LX2160A-RDB imported profile (`fsl_lx2160ardb`)

## Measured profile

The imported Renode target is narrowed to one Cortex-A72 processing element on
the AArch64 backend, the imported 2 GiB DDR window at `0x80000000`, and the
PL011 console UART at `0x021C0000` (descriptor id `uart0`) using the generic
UART engine with the PL011 register layout: polled TX through `UARTFR`/`UARTDR`.
The other A72 cores, the generic timer, the second PL011 at `0x021D0000` and
the Vybrid-style I2C controller from the import are omitted.

An earlier report on this page described a 2026-10-18 run of a Thumb
Cortex-M fixture against this descriptor. That run did not execute A72 code
and is superseded.

## Smoke evidence

The exact imported system ran the committed AArch64 fixture
(`tests/fixtures/lx2160a-a72-uart-smoke.elf`, built by
[`scripts/build_lx2160a_a72_uart_smoke.sh`](../../scripts/build_lx2160a_a72_uart_smoke.sh)).
It sets `UARTCR.UARTEN|TXE`, polls `UARTFR.TXFF`, writes `LX2160A A72 UART
SMOKE` to `UARTDR`, then spins in an integer loop. The smoke asserts the
banner in the captured UART output and the bounded `max_steps` stop after
100,000 instructions. With the UART removed from the descriptor the same
fixture faults on its first UART access (`0x021C0030`, 0/2 checks), so the
banner assertion depends on the model.

This does not test interrupts, baud timing, RX, the GIC, timers, multicore
boot, TF-A/U-Boot, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/fsl_lx2160ardb/` contains
the per-target metric, JUnit result, simulator result, UART log, snapshot and
run log.
