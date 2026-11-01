# Curiosity PIC32MZ EF 2.0

The **Microchip Curiosity PIC32MZ EF 2.0 Development Board** (DM320209) with
a **PIC32MZ2048EFM144**: a PIC32MZ EF MCU (MIPS32 M5150 core with FPU,
200 MHz, 2 MiB program flash, 512 KiB RAM). It is LabWired's **first MIPS
part**.

LabWired runs it on its MIPS core (MIPS32 release 2 with branch delay slots,
CP0, eight shadow register sets, and the PIC32MZ interrupt controller in the
core), with UART2 as the interrupt-driven console and Timer1 as the system
tick. **Unmodified upstream Apache NuttX** (apache/nuttx 7956039dfd) for an
out-of-tree Curiosity board starts at the reset vector 0xBFC00000:

| Script | Firmware | Result |
| :--- | :--- | :--- |
| `nuttx-nsh.yaml` | NuttShell | prompt on UART2; `hello` typed on the console runs and returns |
| `nuttx-ostest.yaml` (manual) | `testing/ostest` | 34 test groups, 0 errors, `ostest_main: Exiting with status 0` |

!!! warning "Not claimed yet"
    Ports and PPS (no LEDs or switches), the FPU, microMIPS, the DSP ASE,
    the TLB, caches, bus error exceptions, DMA, SPI, I2C and Timers 2-9.
    SIM-DERIVED: no bench part.

Evidence and the full claim table: `examples/curiosity-pic32mz-ef/VALIDATION.md`.
Rebuild the fixtures: `crates/firmware-curiosity-pic32mz-nuttx/build.sh`.
