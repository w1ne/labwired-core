# Enclustra Mars ZX3 imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `mars_zx3` target is narrowed
to one Cortex-A9 PE on the native Cortex-A9 CPU model, a 64 KiB DDR aperture
at `0x00100000`, and PS UART1 (Cadence, UG585) at `0xE0001000` (polled TX only). The 64 KiB aperture starts at UG585's DDR base; it is a simulator limit, not the module's DDR size. Which PS UART the module routes to its console is not asserted; UART1 is the one the smoke exercises.

Not modeled and omitted from the descriptor: OCM and the BootROM, the second A9, SCU, GIC, L2C-310, timers, TTCs, UART0, GEMs, USB, SD, QSPI, SPI, I2C, GPIO, DMA, XADC, SLCR and the programmable logic. Interrupts, baud/framing
timing and RX are outside the smoke.

The earlier report on this page described a 2026-10-18 run of a Thumb
Cortex-M fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run never executed Cortex-A9 code and is withdrawn.

## Smoke evidence

The committed A32 fixture `tests/fixtures/zynq7000-a9-uart-smoke.elf` is linked at
`0x00100000`, injected directly into the DDR aperture, polls `Channel_sts.TXFULL` and writes `ZYNQ7000 A9 UART SMOKE` to the TX FIFO, and then spins.
It ran **100,000 Cortex-A9 instructions** and passed the UART banner and
bounded `max_steps` assertions (JUnit: 3 cases, 0 failures). It does not boot
through a boot ROM or exercise MMU, caches, interrupts or multicore.

The exact system manifest is `configs/systems/onboarding/mars_zx3.yaml`, the smoke
script is `examples/renode-onboarding/mars_zx3-a9-uart-smoke.yaml`, and the build
recipe is [`scripts/build_cortex_a9_uart_smokes.sh`](../../scripts/build_cortex_a9_uart_smokes.sh).
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/mars_zx3/`.

This is local CLI simulation evidence for the smoke assertions, not whole-SoC
coverage, Renode equivalence, or hardware confirmation. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
