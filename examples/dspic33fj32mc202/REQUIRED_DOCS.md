# Required Documents — dsPIC33FJ32MC202

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS70283H | dsPIC33FJ32MC202/204 and dsPIC33FJ16MC304 Data Sheet | Memory map, UART1 SFRs (`U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224`), pin/package differences, clock facts |
| DS70000157G | 16-Bit MCU and DSC Programmer's Reference Manual | Instruction encodings used by the shared PIC24 core |

The 202 shares the dsPIC33FJ32MC204's core and memory; the 204's
[`docs/pic24-m0-reference.md`](../../docs/pic24-m0-reference.md) covers the
ELF layout, ISA, DSP MAC, MCPWM/QEI and ADC/SPI/I2C models that apply to the
shared silicon.
