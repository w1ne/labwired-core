# Required Documents — PIC16F877A

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS39582B | PIC16F87XA Data Sheet | Memory organisation (program pages, register file map Figure 2-3, shared 0x70-0x7F), STATUS/PCL/PCLATH/stack, I/O ports, A/D module (section 11: ADCON0/ADCON1 Registers 11-1/11-2, TAD Table 11-1, Figures 11-1/11-3/11-4, Sleep section 11.5), USART (TXSTA/RCSTA/TXREG/SPBRG, PIR1.TXIF), interrupts (INTCON/PIE1/PIR1), reset values (Table 14-6), instruction set summary (Table 15-2) |
| SDCC 4.4.0 manual | Small Device C Compiler, pic14 port | Code-pointer (`RETLW` table) and shared-bank conventions |
| gputils 1.4.0 | `16f877a_g.lkr`, `p16f877a.inc` | Link map and register names for the committed image |
