# Renode onboarding evidence: andes_ae350_n25

The old generated report for this descriptor was a structural check only (no
firmware run) and listed `uart1` while the chip descriptor mapped no
peripherals. This report replaces that claim with a smoke run against the
imported Andes AE350 N25 system and its CPU/DRAM/uart1 profile.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-andes-ae350-n25-smoke --features ae350-uart`, RV32IM,
  100,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IM MUL result, the idle LSR (0x60, THRE|TEMT), the
  LCR/DLAB divisor readback (DLL 0x41, DLM 0x01, LCR 0x03), the firmware pass
  word, the polled UART banner `AE350 N25 uart1 ok`, and the stop reason (6/6
  passed).
- Evidence: `docs/coverage/renode-onboarding/metrics/andes_ae350_n25/`
  (build/run logs, UART output, JUnit, result, and snapshot).

The earlier CPU/memory-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/andes_ae350_n25/2026-11-08-before-uart1-smoke/`.
A negative control with `uart1` removed fails 1/6 with a memory access
violation at the LCR (0xF030002C):
`docs/coverage/renode-onboarding/attempt-history/andes_ae350_n25/2026-11-08-uart1-negative-control/`.

## Profile boundary

The imported chip profile includes the generic RV32IM instruction path, a
128 KiB slice of DRAM at `0x0`, and `uart1` at `0xF0300020` (Renode
`cpus/andes_ae350_n25.repl`, NS16550 with `wideRegisters: true`). `uart1` uses
this engine's 4-byte-stride 16550 register map (`dw_apb_uart` profile); that
selects a register layout and does not claim a Synopsys DesignWare UART.

The ATCUART100 ID/OSC words below `0xF0300020`, UART interrupts, FIFOs and
baud timing, the PLIC, the Andes `xandes`, floating-point and compressed
extensions, and the rest of the 1 GiB DRAM are not claimed. Simulation assumes
an ideal VDD rail; power-on and brown-out behavior were not exercised. This is
repository CLI simulation evidence, not hardware confirmation or a
silicon-wide peripheral claim.
