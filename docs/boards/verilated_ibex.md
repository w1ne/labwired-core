# Renode onboarding evidence: verilated_ibex

The imported board descriptor remains `verified: false`. Renode's
[platform file](https://github.com/renode/renode/blob/master/platforms/cpus/verilated/verilated_ibex.repl) defines a co-simulated Verilated Ibex core (`CoSimulated.CoSimulatedRiscV32`, `cpuType: ibex`), a 256 MiB stack window at `0xF0000000`, a 32 KiB ROM at `0x00000000`, 8 KiB of
SRAM at `0x01000000`, 256 MiB of main RAM at `0x40000000`, a
`LiteX_SoC_Controller` at `0x82000000`, a `LiteX_UART` at `0x82002000`, and
a `Timers.LiteX_Timer_CSR32` timer0 at `0x82002800` (100 MHz). The timer's
32-bit CSR layout is `load` 0x00, `reload` 0x04, `en` 0x08,
`update_value` 0x0C, `value` 0x10, `ev_status` 0x14, `ev_pending` 0x18
(W1C), `ev_enable` 0x1C.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-litex-ibex-smoke`, RV32IMC, 100,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the banner, then in order the RV32IMC MUL result, `ev_pending`
  clearing on a written 1 (`timer=cleared`), `load` read back as 0x00004E20,
  a latched `value` strictly inside the 20,000-cycle one-shot
  (`timer=counting`), the zero-event level low right after enable, the
  `ev_pending` latch after the countdown (`timer=zero`), and the stop reason.
- Evidence: [`docs/coverage/renode-onboarding/metrics/verilated_ibex/`](../coverage/renode-onboarding/metrics/verilated_ibex/)
  (build/run logs, JUnit, result, and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

## Correction (2026-11-08)

Until this date the chip descriptor declared timer0 as
`litex_timer_csr32`. That is not a LabWired model type: the fuzzy
`contains("timer")` rule resolved it to the STM32 general-purpose timer,
and the earlier smoke passed only because its `ev_pending` write landed in
STM32 CCMR1 (+0x18), which reads the 1 back. The descriptor now uses
`litex_timer` (32-bit CSR layout), and the smoke checks W1C, `load`
readback, and the `value` latch, which the STM32 model fails
(`timer=pending_kept`, `load=00000220`, `value=00000000`). The earlier
measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/verilated_ibex/2026-11-08-before-litex-timer-model/`.
The fallback was demonstrated on the shared firmware through `litex_ibex`,
whose chip profile was identical:
`docs/coverage/renode-onboarding/attempt-history/litex_ibex/2026-11-08-stm32-timer-fallback-exposed/`.

## Profile boundary

LabWired runs this firmware on its own RV32IMC interpreter. It does **not**
model Renode's co-simulated Verilated Ibex RTL; the result says the SoC map
and peripheral models are consistent, not that the RTL core was exercised.
The profile selects SRAM, a 1 MiB direct-ELF load window at `0x20000000` (a
LabWired loading convenience, not a Renode region), the LiteX UART, and timer0
as `litex_timer`.

Not claimed: the Verilated CPU, the SoC controller, the ROM boot flow, main
RAM, the stack window, the silenced DDR PHY/SDRAM windows, and interrupts.
Renode routes the UART to `cpu@0` and timer0 to `cpu@7`; the chip descriptor's
IRQ numbers (2 and 1) do not match and are not exercised — the smoke polls.
This is repository CLI simulation evidence, not hardware confirmation.
