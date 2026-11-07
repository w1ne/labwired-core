# Arm Versatile/PB imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `versatile` target is
narrowed to one ARM926EJ-S on the native ARM926 CPU model (ARMv5TEJ, MIDR
`0x41069265`), a 64 KiB SDRAM aperture at `0x00000000`, and UART0 (PL011) at
`0x101F1000` (polled TX only). The 64 KiB aperture starts at the SDRAM base; it
is a simulator limit, not the SDRAM size.

Not modeled and omitted from the descriptor: the VIC/SIC interrupt
controllers, SP804 timers, system controller, UART1, CLCD (PL110), SMC91x
Ethernet, KMI (PL050), PCI/ISP1761 USB with its hub, mouse and keyboard, and
flash. Thumb execution on this core is refused by the adapter, as is the ARMv5
CP15 wait-for-interrupt. Interrupts, baud/framing timing and RX are outside
the smoke.

The earlier report on this page described a 2026-10-18 run of a Thumb
Cortex-M fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run never executed ARM926 code and is withdrawn.

## Smoke evidence

The committed A32 fixture `tests/fixtures/versatile-arm926-uart-smoke.elf` is
linked at `0x00000000`, injected directly into the SDRAM aperture, sets
`UARTCR.UARTEN|TXE`, polls `UARTFR.TXFF` and writes
`VERSATILE ARM926 UART SMOKE` to `UARTDR`, and then spins. It ran **100,000
ARM926EJ-S instructions** and passed the UART banner and bounded `max_steps`
assertions (JUnit: 3 cases, 0 failures). With UART0 removed from the
descriptor, the same fixture stops with a memory violation at `0x101F1030`
(`UARTCR`) and passes 0/2 checks. The fixture does not boot through a boot ROM
or exercise MMU, caches, interrupts or Thumb code.

The exact system manifest is `configs/systems/onboarding/versatile.yaml`, the
smoke script is `examples/renode-onboarding/versatile-arm926-uart-smoke.yaml`,
and the build recipe is
[`scripts/build_arm926_uart_smoke.sh`](../../scripts/build_arm926_uart_smoke.sh).
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/versatile/`.

This is local CLI simulation evidence for the smoke assertions, not whole-SoC
coverage, Renode equivalence, or hardware confirmation. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
