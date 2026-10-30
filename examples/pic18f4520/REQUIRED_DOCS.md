# Required Documents — PIC18F4520

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS39631E | PIC18F2420/2520/4420/4520 Data Sheet | Memory organisation (program/data maps, Access Bank, SFR map Table 5-1), I/O ports, EUSART (TXSTA/RCSTA/TXREG/SPBRG, PIR1.TXIF), interrupts (INTCON/INTCON3/RCON), reset values, instruction set summary and per-instruction examples |
| SDCC 4.4.0 manual | Small Device C Compiler, pic16 port | Calling convention and software stack (FSR1/FSR2), generic-pointer layout |
| gputils 1.4.0 | `18f4520_g.lkr`, `p18f4520.inc` | Link map and register names for the committed image |

The shared core is documented in
[`docs/PIC18-CORE-PROGRESS.md`](../../docs/PIC18-CORE-PROGRESS.md).
