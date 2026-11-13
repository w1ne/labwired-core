# Required Documents — Curiosity PIC32MZ EF 2.0

Sources used by this example (retrieved 2026-11-01):

| Doc | Used for |
| :--- | :--- |
| MIPS32 Architecture For Programmers Vol. II: The MIPS32 Instruction Set (MD00086) | Instruction semantics: delay slots, branch-likely, LWL/LWR/SWL/SWR, LL/SC, traps, the release 2 additions |
| MIPS32 Architecture For Programmers Vol. III: The MIPS32 Privileged Resource Architecture (MD00090) | CP0 registers and reset state, exception vectors (BEV, EBase, IV, VS), EIC interrupt mode, shadow register sets, ERET |
| PIC32MZ Embedded Connectivity with Floating Point Unit (EF) Family data sheet (DS60001320) | Memory map, CPU configuration, the interrupt controller (IFS/IEC/IPC/OFF, INTSTAT, PRISS, single and multi vector), UART, Timer1/Type B timers, DEVCFG0.BOOTISA |
| Curiosity PIC32MZ EF 2.0 Development Board User's Guide (DS70005400) | Board: PIC32MZ2048EFM144, UART2 (TX RG9, RX RE9) to the PKoB4 virtual COM port, Y1 12 MHz oscillator, LEDs |
| Microchip MPLAB Harmony 3 `bsp/boards/pic32mz_ef_curiosity_v2/config/bsp.py` | LED and switch pins (LED1 RJ7, LED2 RK7, LED3 RJ3), ICESEL PGx1 |
| Apache NuttX `arch/mips/src/pic32mz` (7956039dfd) | SFR block bases and register offsets, the EVIC/IRQ numbering, the startup and context-switch paths the model must satisfy |

Open items: the M5150 PRId revision field and the Config1 cache geometry are
written from the PIC32MZ EF configuration, not read back from silicon.
