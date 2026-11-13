# Renode onboarding evidence: riscv_verilated_uartlite

The imported board descriptor remains `verified: false`. Renode defines an
RV32G core, a PLIC at `0x40000000`, a CLINT at `0x44000000`, 100 MiB of RAM at
`0x60000000`, and a `CoSimulated.CoSimulatedUART` at `0x70000000` (+0x100) in
its [platform file](https://github.com/renode/renode/blob/master/platforms/cpus/verilated/riscv_verilated_uartlite.repl).
That UART is a Verilated RTL block, not a C# model; its register map comes from
[`samples/uartlite/top.v`](https://github.com/antmicro/renode-verilator-integration/blob/master/samples/uartlite/top.v)
in antmicro/renode-verilator-integration.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-riscv-verilated-uartlite-smoke`, RV32IM, 100,000
  instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IM MUL result, CTRL (0x8) reading back 0 after a FIFO
  reset write (it is write-only in the RTL), the idle STAT (0x04, tx empty),
  the firmware pass word (also checks RX valid clear and RX data 0), the
  STAT-polled UART banner `UARTLite cosim ok`, and the stop reason (6/6
  passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/riscv_verilated_uartlite/`](../coverage/renode-onboarding/metrics/riscv_verilated_uartlite/)
  (build/run logs, UART output, JUnit, result, and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The earlier CPU/RAM-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/riscv_verilated_uartlite/2026-11-08-before-uart-smoke/`.
A negative control with `uart` removed fails 1/6 with a memory access
violation at CTRL (0x70000008):
`docs/coverage/renode-onboarding/attempt-history/riscv_verilated_uartlite/2026-11-08-uart-negative-control/`.

## Profile boundary

The LabWired onboarding profile selects the generic `rv32im` CPU path, the
100 MiB RAM window, and `uart` on the vendor-neutral `custom` UART map:
RX data 0x0, TX data 0x4, STAT 0xC. The RTL's map is **not** the Xilinx AXI
UARTLite map (STAT and CTRL are swapped), and this profile follows the RTL.
STAT idles at 0x04 and reads 0x03 (rx valid | rx full) while an RX byte is
queued; CTRL is not stored, so STAT bit4 (irq enabled) stays 0 and CTRL's
FIFO-reset bits have no effect.

Not claimed: the full RV32G ISA, the PLIC and CLINT, UART interrupts, the
STAT tx-busy window (TX completes instantly here), overrun/frame errors, and
baud timing (the RTL fixes 115200 at 100 MHz). Simulation assumes an ideal
VDD rail; power-on and brown-out behavior were not exercised. This is
repository CLI simulation evidence, not hardware confirmation or a
co-simulation of the RTL.
