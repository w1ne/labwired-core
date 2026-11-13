# Validation Report: `litex_linux_vexriscv_sdcard`

## Measured profile

The imported profile was mislabeled as ARM and included generated peripherals
that are not modeled. It now declares one RV32IMA CPU, the upstream 256 MiB DDR
window at `0x40000000`, and the implemented LiteX UART at `0xf0001000`.

The profile omits Linux boot, SD-card, Ethernet, GPIO, SPI, I2C, MMCM, and other
unexercised devices. The 32 KiB firmware link window is a smoke image subset of
the 256 MiB DDR mapping.

## Local LabWired smoke

The exact imported system ran the committed bare-metal RV32IM fixture for
100,000 instructions (400,000 cycles) and stopped at `max_steps`. The firmware
printed both expected UART markers, and both smoke assertions passed. It does
not boot Linux or access the SD card.

Evidence: [`metrics/litex_linux_vexriscv_sdcard/`](../coverage/renode-onboarding/metrics/litex_linux_vexriscv_sdcard/).
The first passing run used a 256 MiB linker stack reservation and is retained
under [`attempt-history/`](../coverage/renode-onboarding/attempt-history/litex_linux_vexriscv_sdcard/2026-11-06-initial-256m-linker-window/); the final measurement uses a 32 KiB link window.

The run output records two unmapped reads at `0x0` and `0x4` with first PC `0x0`.
They did not block the UART assertions, but remain part of the recorded fidelity
output. The simulator also assumes an ideal VDD rail.

## Scope

This is a local LabWired CLI smoke against a bounded imported profile. It is not
execution in Renode, a hosted `labwired_verify` verdict, Linux boot evidence,
SD-card validation, or hardware confirmation.

Upstream platform description:
[`litex_linux_vexriscv_sdcard.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/litex_linux_vexriscv_sdcard.repl).
