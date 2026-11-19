# Renode onboarding evidence: fomu

The imported board descriptor remains `verified: false`. Renode defines a
VexRiscv (`rv32im_zicsr_zifencei`), an 8 KiB ROM at `0x00000000`, 128 KiB of
SRAM at `0x10000000`, a `Timers.LiteX_Timer` timer0 at `0x60002800`,
ValentyUSB at `0x60004800`, a PicoRV SPI controller with a Micron MT25Q flash,
and tagged CTRL/TOUCH/REBOOT/RGB/VERSION pages in its
[platform file](https://github.com/renode/renode/blob/master/platforms/cpus/fomu.repl).
The CSR layout follows Foboot's generated
[`csr.h`](https://github.com/im-tomu/foboot/blob/637bbf10fa9a0b7f52cbd459d80c8c7532d56a18/sw/include/generated/csr.h),
which uses `csr_data_width = 8`. Renode maps no UART for this board.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-fomu-smoke --features timer0`, RV32IM, 100,000
  instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IM MUL result, a 32-bit `load` written and read back
  across its four byte-wide CSR slots (0x00012345), the six-bit check word
  (0x3F: idle event level/pending clear after staging, `en` readback, a
  latched `value` strictly inside the 2000-cycle one-shot, the zero-event
  level and pending latch after polling, and `ev_pending` ignoring a written 0
  and clearing on a written 1), the firmware pass word 0xF0A0003F, and the
  stop reason (5/5 passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/fomu/`](../coverage/renode-onboarding/metrics/fomu/)
  (build/run logs, JUnit, result, and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The earlier CPU/RAM-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/fomu/2026-11-08-before-timer0-smoke/`.
A negative control with `timer0` removed fails 1/5 with a memory access
violation at `en` (0x60002820):
`docs/coverage/renode-onboarding/attempt-history/fomu/2026-11-08-timer0-negative-control/`.

## Profile boundary

The LabWired onboarding profile selects the generic `rv32im` CPU path, the
ROM and SRAM windows (no shadow aliases), and timer0 as `litex_timer` with
`csr_data_width: 8`. The timer counts SoC cycles of the simulated core; the
board's 12 MHz timer clock is not modelled as a separate rate.

Not claimed: timer0's IRQ (Renode `-> cpu@0`; the smoke polls the event
CSRs), the 0x8000_0000/0xE000_0000 shadow aliases, ValentyUSB, the SPI flash
controller and MT25Q, the RGB/touch/reboot/version blocks, the VexRiscv debug
window, and the Foboot boot flow. Simulation assumes an ideal VDD rail;
power-on and brown-out behavior were not exercised. This is repository CLI
simulation evidence, not hardware confirmation.
