# Required Documents — dsPIC33FJ32MC204

Authoritative sources used by this example (retrieved 2026-10-18):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS70283H | dsPIC33FJ32MC202/204 and dsPIC33FJ16MC304 Data Sheet | Memory map (Fig. 4-1/4-3), SFR maps (Tables 4-11/4-13/4-15), vectors (7-1), pins (1-1), clock/PLL facts |
| DS70000157G | 16-Bit MCU and DSC Programmer's Reference Manual | Instruction encodings for the modelled subset (base ISA + DSP MAC class) |
| DS70183D | dsPIC33F/PIC24H Family Reference Manual, Section 16 — ADC | ADC timing (TAD/SAMC), SSRC, SMPI, buffer-fill and FORM formats |
| DS70206 | dsPIC33F/PIC24H Family Reference Manual, Section 18 — SPI | SPI master/slave operation, prescalers, error flags |
| DS70000195 | dsPIC33F/PIC24H Family Reference Manual, Section 19 — I²C | I²C master/slave state machine, SEN/PEN/RCEN/ACKEN |
| SBOS397I | TMP102 temperature sensor data sheet | TMP102 pointer/register model (attached on I²C1) |

Encodings and addresses are transcribed in `docs/pic24-m0-reference.md`
(§3.1 ELF layout, §3.2 DSP MAC, §3.3 MCPWM/QEI, §3.4 ADC/SPI/I²C); the
engine's decode lives in `crates/core/src/cpu/pic24.rs` with the instruction
table in its module docs.

The XC16 ELF layout is no longer provisional: it was verified against XC16
v1.70 output (4-byte instruction slots, PC-unit addresses, reset GOTO at PC 0,
IVT holding 24-bit ISR addresses as data) and is covered by
`crates/core/tests/pic24_xc16_elf_boot.rs`.
