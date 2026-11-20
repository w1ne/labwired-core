# Arm Versatile Express imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `vexpress` target is narrowed
to one Cortex-A9 PE on the native Cortex-A9 CPU model, a 64 KiB DRAM aperture
at `0x80000000`, and motherboard UART0 (PL011) at `0x10009000` (polled TX only). The 64 KiB aperture starts at the imported DRAM base; it is a simulator limit, not the DRAM size.

Not modeled and omitted from the descriptor: the other three A9s, SCU, GIC, private/global timers, SP804 timers, SP810, UART1–UART3, LAN9118, MMCI, KMI, CLCD and NOR flash. Interrupts, baud/framing
timing and RX are outside the smoke.

The earlier report on this page described a 2026-10-18 run of a Thumb
Cortex-M fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run never executed Cortex-A9 code and is withdrawn.

## Smoke evidence

The committed A32 fixture `tests/fixtures/vexpress-a9-uart-smoke.elf` is linked at
`0x80000000`, injected directly into the DRAM aperture, sets `UARTCR.UARTEN|TXE`, polls `UARTFR.TXFF` and writes `VEXPRESS A9 UART SMOKE` to `UARTDR`, and then spins.
It ran **100,000 Cortex-A9 instructions** and passed the UART banner and
bounded `max_steps` assertions (JUnit: 3 cases, 0 failures). It does not boot
through a boot ROM or exercise MMU, caches, interrupts or multicore.

The exact system manifest is `configs/systems/onboarding/vexpress.yaml`, the smoke
script is `examples/renode-onboarding/vexpress-a9-uart-smoke.yaml`, and the build
recipe is [`scripts/build_cortex_a9_uart_smokes.sh`](../../scripts/build_cortex_a9_uart_smokes.sh).
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/vexpress/`.

This is local CLI simulation evidence for the smoke assertions, not whole-SoC
coverage, Renode equivalence, or hardware confirmation. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
