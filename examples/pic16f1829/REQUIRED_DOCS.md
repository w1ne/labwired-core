# Required Documents — PIC16F1829

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS40001440E | PIC16(L)F1825/9 Data Sheet | Enhanced mid-range core (chapter 3: banks, core registers, common RAM, linear and flash FSR spaces, 16-level stack with STKPTR/TOS, STVREN), interrupts and automatic context save (chapter 7), oscillator status (OSCSTAT), I/O ports with LATx, TMR0 and OPTION_REG, EUSART (TXSTA/RCSTA/TXREG/SPBRG, PIR1.TXIF), PCON reset flags, special function register summary (Table 3-8), instruction set summary (Table 29-3) |
| DS40001804 | Curiosity Development Board User's Guide | Board: 20-pin socket, LED D4 on RA5 |
| SDCC 4.4.0 manual | Small Device C Compiler, pic14 port | Enhanced-core interrupt handlers (no software context save), code-pointer conventions |
| gputils 1.4.0 | `16f1829_g.lkr`, `p16f1829.inc` | Link map and register names for the committed image |
