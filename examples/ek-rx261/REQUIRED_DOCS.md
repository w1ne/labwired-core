# Required Documents — EK-RX261

Sources used by this example (retrieved 2026-11-01):

| Doc | Used for |
| :--- | :--- |
| RX Family RXv3 Instruction Set Architecture User's Manual: Software (R01US0316) | Instruction semantics and flags, PSW/USP/ISP/INTB/EXTB, exception and interrupt entry/return, accumulator and FPU behaviour |
| RX261 Group User's Manual: Hardware (R01UH1045) | Memory map, ICU (IR/IER/IPR/SWINTR/FIR, vector table), CMT, SCI, clock generation (OSCOVFSR, OPCCR), FLASH MEMWAITR |
| EK-RX261 v1 User's Manual | Board: R5F52618BGFP, SCI6 to the on-board debugger's virtual COM port |
| Renesas RX Driver Package (hal_renesas a279c14e196c) `iodefine.h`, `mcu_clocks.c` | Register offsets (checked by compiling `offsetof` with the RX GCC), the start-up handshakes |
| Zephyr v4.3.0 `dts/rx/renesas/rx261-common.dtsi`, `boards/renesas/ek_rx261`, `arch/rx` | Peripheral bases and vectors (SCI6 226-229, CMT0 28, CMT1 29), console on SCI6, CMT at 4 MHz, the low-vector IPR map, the context-switch path |
| binutils 2.43 `opcodes/rx-decode.opc`, `rx-dis.c`; QEMU `target/rx/insns.decode` | Encoding cross-checks only (the decoder is written independently; every unit-test encoding comes from the assembler) |

Open items: RX261 per-vector IPR sharing beyond what Zephyr uses is taken from
Zephyr's `irq_manage.c` table, not yet transcribed from the hardware manual's
vector table.
