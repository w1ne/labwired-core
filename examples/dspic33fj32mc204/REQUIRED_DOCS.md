# Required Documents — dsPIC33FJ32MC204

Authoritative sources used by this example (retrieved 2026-10-18):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS70283H | dsPIC33FJ32MC202/204 and dsPIC33FJ16MC304 Data Sheet | Memory map (Fig. 4-1/4-3), UART1 SFR addresses (`U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224`), clock/PLL facts |
| DS70000157G | 16-Bit MCU and DSC Programmer's Reference Manual | Instruction encodings for the modelled subset (`NOP`, `MOV`, `BRA`) |

Encodings and addresses are transcribed in `docs/pic24-m0-reference.md`; the
engine's decode lives in `crates/core/src/cpu/pic24.rs` with the instruction
table in its module docs.

Open item: replace the provisional ELF/instruction-slot mapping with the real
XC16 ELF layout when a toolchain is available (tracked in the M0 notes).
