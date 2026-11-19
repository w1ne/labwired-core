# Renode onboarding evidence: core-v-mcu

The imported board descriptor remains `verified: false`. Renode's [platform file](https://github.com/renode/renode/blob/master/platforms/cpus/core-v-mcu.repl) defines a CV32E40P (RV32IMFC, Priv 1.11), an 8 KiB boot ROM at `0x1A000000`, 512 KiB of SRAM at `0x1C000000` (aliased at 0), the PULP interrupt/event controllers, uDMA UART/SPI/I2C/camera, a PULP timer, APB GPIO, I2S and a `UART.PULP_STDOUT` console at `0x1A10FF80`.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-cv32e40p-smoke`, RV32IMC, 100,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IMC MUL result (0x143), the STD_OUT readback (0, over
  a 0xDEADBEEF sentinel), the firmware pass word 0xC0DE5701, the banner
  `CORE-V stdout ok` on the `stdout` console (each byte written as a 32-bit
  word with non-zero reserved bits), and the stop reason (5/5 passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/core-v-mcu/`](../coverage/renode-onboarding/metrics/core-v-mcu/)
  (build/run logs, JUnit, result, UART log and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The earlier CPU/RAM-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/core-v-mcu/2026-11-08-before-stdout-smoke/`.
A negative control with `stdout` removed fails 1/5 with a memory access violation at 0x1A10FF80: `docs/coverage/renode-onboarding/attempt-history/core-v-mcu/2026-11-08-stdout-negative-control/`.

## Profile boundary

The LabWired onboarding profile selects the generic `rv32imc` CPU path (no F
extension, no PULP/XCV custom instructions), the boot ROM and SRAM windows
(no SRAM alias at 0), and `stdout` as a `uart` with a custom map: STD_OUT at
offset 0, write transmits the low byte, reads return 0, matching Renode's
`PULP_STDOUT` (renode-infrastructure
`Peripherals/UART/PULP_STDOUT.cs`).

Not claimed: the uDMA UART at 0x1A102100, uDMA SPI/I2C/camera, the PULP
timer, APB GPIO, I2S, the PULP interrupt and event controllers, the FLL,
SoC control and the other tagged windows, and the Hart ID 0x3E0. Simulation
assumes an ideal VDD rail; power-on and brown-out behavior were not
exercised. This is repository CLI simulation evidence, not hardware
confirmation.
