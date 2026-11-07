# Alif Ensemble E8 imported profile

## Measured profile

The imported Alif E8 descriptor is narrowed to one Cortex-M55 core with
256 KiB of local ITCM at `0x00000000` and 1 MiB of local DTCM at
`0x20000000`. The descriptor's `flash` slot serves as the executable ITCM load
window; it does not model the device's MRAM or boot copy process. Alif documents
the E8 device as having Cortex-M55 cores and these local memory windows
([E8 series](https://alifsemi.com/ensemble-e8-series/),
[memory map](https://swrm.alifsemi.com/Content/2.6%20Shared%20Memory.htm?TocPath=Overview+of+the+Device+System+Architecture%7C_____5)).

UART0-7 are mapped at `0x49018000 + n*0x1000`, the addresses in Renode's
[Alif E8 platform description](https://github.com/renode/renode/blob/master/platforms/cpus/alif_ae822fa0e5597bs0.repl).
Renode maps them as `UART.NS16550` with `wideRegisters: true`. LabWired uses
its `dw_apb_uart` profile for them: a Synopsys DW_apb_uart, which is a 16550
with a 4-byte register stride, and DLL/DLM sit behind LCR.DLAB. No UART
interrupt is declared, so NVIC lines 124-131 are not claimed.

These are not mapped, because LabWired has no model for them: the DesignWare
APB GPIO ports (gpio0-14, lpgpio), the DesignWare I2C0-3, CAST CAN-FD, SRAM0/1,
MRAM, and the clock and pinctrl windows. The second Cortex-M55, both Cortex-A32
cores, Helium/MVE and boot firmware are also outside the profile.

## Smoke evidence

The smoke firmware (`examples/renode-onboarding/alif-e8-m55-cpu-smoke.S`,
built by `scripts/build_alif_e8_m55_cpu_smoke.sh`) is loaded into ITCM. It
sets UART0 LCR to `0x83` (DLAB, 8N1) and reads it back. It writes the divisor
latches (DLL `0x1B`, DLM `0x02`), then gives UART7 its own divisor (`0x05`).
It reads both back, clears DLAB, and checks the LSR idle flags (THRE|TEMT). It
then transmits the results on UART0, polling LSR.THRE. The exact imported
system ran 100,000 instructions and passed 4/4 assertions: the UART0 lines
`ALIF E8 M55 SMOKE`, `uart0 dl=021b`, `uart7 dll=05` and `pass=1f`, the result
word `0xA1E8001F` at `0x200FFFF0`, and the `max_steps` stop.

Negative control: the same firmware on the previous CPU-only chip profile fails
0/4. It takes a BusFault at UART0 LCR (`0x4901800C`).

The fixture is assembled for a Cortex-M33-compatible Thumb subset. This is
local LabWired CLI evidence. It does not cover the full Cortex-M55 ISA,
Helium, interrupts, FIFOs, baud timing, DMA, multicore behavior, boot
firmware, Renode execution or physical hardware.

Evidence: `docs/coverage/renode-onboarding/metrics/alif_ae822fa0e5597bs0/`
contains the per-target metric, JUnit result, simulator result, snapshot, and
run log.
