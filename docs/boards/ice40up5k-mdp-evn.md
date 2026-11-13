# iCE40 UltraPlus MDP Renode scope

Renode's board file loads the Fomu VexRiscv profile. Its CPU is configured as
`rv32im_zicsr_zifencei`, with an 8 KiB ROM at `0x0`, 128 KiB SRAM at
`0x10000000`, and a `Timers.LiteX_Timer` timer0 at `0x60002800`
([board REPL](https://github.com/renode/renode/blob/master/platforms/boards/ice40up5k-mdp-evn.repl),
[CPU REPL](https://github.com/renode/renode/blob/master/platforms/cpus/fomu.repl)).
On top of that the board adds a `LiteX_I2C` controller at `0x60007800` with
BMP180, LSM330, LIS2DS12 and LSM303DLHC sensors. Its CSR layout follows
Foboot's generated
[`csr.h`](https://github.com/im-tomu/foboot/blob/637bbf10fa9a0b7f52cbd459d80c8c7532d56a18/sw/include/generated/csr.h)
(`csr_data_width = 8`).

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-fomu-smoke --features timer0` (the same image as the
  Fomu row, because the CPU profile is shared), RV32IM, 100,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IM MUL result, a 32-bit `load` written and read back
  across its four byte-wide CSR slots (0x00012345), the six-bit check word
  (0x3F: idle event level/pending clear after staging, `en` readback, a
  latched `value` strictly inside the 2000-cycle one-shot, the zero-event
  level and pending latch after polling, and `ev_pending` ignoring a written 0
  and clearing on a written 1), the firmware pass word 0xF0A0003F, and the
  stop reason (5/5 passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/ice40up5k-mdp-evn/`](../coverage/renode-onboarding/metrics/ice40up5k-mdp-evn/)
  (build/run logs, JUnit, result, and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The earlier CPU/RAM-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/ice40up5k-mdp-evn/2026-11-08-before-timer0-smoke/`.
A negative control with `timer0` removed is at
`docs/coverage/renode-onboarding/attempt-history/ice40up5k-mdp-evn/2026-11-08-timer0-negative-control/`.

## Profile boundary

The LabWired onboarding profile selects the generic `rv32im` CPU path, the
ROM and SRAM windows (no shadow aliases), and timer0 as `litex_timer` with
`csr_data_width: 8`. The timer counts SoC cycles of the simulated core; the
12 MHz timer clock is not modelled as a separate rate.

Not claimed: timer0's IRQ (Renode `-> cpu@0`; the smoke polls the event
CSRs), the LiteX I2C controller and its four sensors (no `LiteX_I2C` model in
this engine, so the window stays unmapped), the shadow aliases, ValentyUSB,
the SPI flash controller, the Fomu CTRL/TOUCH/REBOOT/RGB/VERSION blocks, and
reset-vector boot. Simulation assumes an ideal VDD rail. This is repository
CLI simulation evidence, not hardware confirmation.
