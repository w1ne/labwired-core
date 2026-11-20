# NVIDIA Tegra 2 imported profile

## Measured profile

LabWired's onboarding profile for the imported Renode `tegra2` target is narrowed
to one Cortex-A9 PE on the native Cortex-A9 CPU model, a 64 KiB SDRAM aperture
at `0x00000000`, and UARTA (16550, 4-byte register stride) at `0x70006000` (polled TX only). The 64 KiB aperture starts at the SDRAM base; it is a simulator limit. UARTA uses the engine's existing 32-bit, 4-byte-stride 16550 map (`dw_apb_uart` profile) to match Tegra's reg-shift of 2; this does not identify the Tegra UART as a DesignWare device.

Not modeled and omitted from the descriptor: the boot ROM, the AVP (ARM7), the second A9, SCU, GIC, timers, clock-and-reset, pinmux, UARTB–UARTE, DMA/host1x, USB, SDMMC and the display/video blocks. Interrupts, baud/framing
timing and RX are outside the smoke.

The earlier report on this page described a 2026-10-18 run of a Thumb
Cortex-M fixture (`uart-ok-thumbv7m.elf`) against the unnarrowed import. That
run never executed Cortex-A9 code and is withdrawn.

## Smoke evidence

The committed A32 fixture `tests/fixtures/tegra-a9-uart-smoke.elf` is linked at
`0x00000000`, injected directly into the SDRAM aperture, polls `LSR.THRE` and writes `TEGRA A9 UART SMOKE` to `THR`, and then spins.
It ran **100,000 Cortex-A9 instructions** and passed the UART banner and
bounded `max_steps` assertions (JUnit: 3 cases, 0 failures). It does not boot
through a boot ROM or exercise MMU, caches, interrupts or multicore.

The exact system manifest is `configs/systems/onboarding/tegra2.yaml`, the smoke
script is `examples/renode-onboarding/tegra2-a9-uart-smoke.yaml`, and the build
recipe is [`scripts/build_cortex_a9_uart_smokes.sh`](../../scripts/build_cortex_a9_uart_smokes.sh).
Per-target metrics, JUnit, UART output, simulator result, snapshot and logs are
under `docs/coverage/renode-onboarding/metrics/tegra2/`.

This is local CLI simulation evidence for the smoke assertions, not whole-SoC
coverage, Renode equivalence, or hardware confirmation. See the
[generated scoreboard](../coverage/renode-onboarding-scoreboard.md) for the
current global totals.
