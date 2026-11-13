# Cortex-A53 console imported profile

## Measured profile

The imported target is narrowed to one Cortex-A53 processing element, the 128 MiB DDR window at
`0x40000000`, and the PL011 UART0 at `0x09000000`, served by the generic UART
engine with the PL011 register layout (polled TX through `UARTFR`/`UARTDR`).
The generic timer and the virtio-mmio console from the imported descriptor are omitted.

## Smoke evidence

The exact imported system ran the committed AArch64 fixture
(`tests/fixtures/virt-a53-pl011-smoke.elf`, built by
[`scripts/build_virt_a53_pl011_smoke.sh`](../../scripts/build_virt_a53_pl011_smoke.sh)).
It checks that `UARTFR` reads `TXFE|RXFE` after reset, enables the UART, writes
`VIRT A53 PL011 SMOKE FR=OK` to `UARTDR` while polling `FR.TXFF`, then spins in
an integer loop. The smoke asserts the banner in the captured UART output and
the bounded `max_steps` stop after 100,000 instructions. A window answering
zeros would print the `FR=BAD` banner instead; with UART0 removed from the
descriptor the fixture faults on its first `UARTFR` read (0/2 checks).

This does not test interrupts, RX, baud timing, the GIC, generic timer,
multicore boot, boot firmware, Renode, or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/cortex_a53_console/` contains the
per-target metric, JUnit result, simulator result, UART log, snapshot, and run
log.
