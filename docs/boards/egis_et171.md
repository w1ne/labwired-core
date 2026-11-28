# Renode onboarding evidence: egis_et171

The imported board descriptor remains `verified: false`. Renode defines an
Andes D25F core with `rv32gc_xandes`, SRAM at `0x00000000` (384 KiB), a 64 KiB
ROM at `0x70000000`, flash at `0x80000000` (1 MiB), and an NS16550 `uart0` at
`0xF0200020` with `wideRegisters: true` in its
[ET171 platform file](https://github.com/renode/renode/blob/master/platforms/cpus/egis_et171.repl).

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-andes-ae350-n25-smoke --features et171-uart`, RV32IM,
  100,000 instructions (shared with the AE350 N25 smoke; only the UART base
  and banner differ).
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IM MUL result, the idle LSR (0x60, THRE|TEMT), the
  LCR/DLAB divisor readback (DLL 0x41, DLM 0x01, LCR 0x03), the firmware pass
  word, the polled UART banner `ET171 uart0 ok`, and the stop reason (6/6
  passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/egis_et171/`](../coverage/renode-onboarding/metrics/egis_et171/)
  (build/run logs, UART output, JUnit, result, and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The earlier CPU/SRAM-only measurement (2026-11-06 configuration, zero
peripheral accesses) is preserved at
`docs/coverage/renode-onboarding/attempt-history/egis_et171/2026-11-08-before-uart0-smoke/`.
A negative control with `uart0` removed fails 1/6 with a memory access
violation at the LCR (0xF020002C):
`docs/coverage/renode-onboarding/attempt-history/egis_et171/2026-11-08-uart0-negative-control/`.

## Profile boundary

The LabWired onboarding profile selects the generic `rv32im` CPU path,
384 KiB of SRAM, the sourced 1 MiB flash window, and `uart0`. `uart0` uses
this engine's 4-byte-stride 16550 register map (`dw_apb_uart` profile); that
selects a register layout and does not claim a Synopsys DesignWare UART.

Not claimed: Renode's reset-vector/ROM flow (the smoke loads the ELF
directly), the D25F's Xandes, compressed, atomic, or floating-point
extensions, the PLICs and machine timer, UART interrupts, FIFOs and baud
timing, and the imported PIT, GPIO, watchdog, SPI, RTC, AOSMU/SMU2
system-control, and crypto blocks. Simulation assumes an ideal VDD rail;
power-on and brown-out behavior were not exercised. This is repository CLI
simulation evidence, not hardware confirmation or a silicon-wide peripheral
claim.
