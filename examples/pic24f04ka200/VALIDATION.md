# Validation — PIC24F04KA200 (L1)

Evidence for the exact 14-pin PIC24F04KA200 descriptor and its UART1 smoke.
There is no silicon capture.

## Status

| Area | State | Evidence |
| :--- | :--- | :--- |
| Chip descriptor | ✅ | `configs/chips/pic24f04ka200.yaml` (DS30009937C memory, SFR, package facts) |
| Geometry | ✅ | `ka_descriptors_match_datasheet_geometry_masks_and_pins` (4 KB flash, 512 B SRAM, A 0x007F / B 0xC310 / no C, 12 pins) |
| System manifest | ✅ | `examples/pic24f04ka200/system.yaml` |
| Boot + UART1 | ✅ | real XC16 v1.70 ELF; `pic24_ka_smoke::pic24f04ka200_uart_smoke` |
| CLI smoke | ✅ | `examples/pic24f04ka200/io-smoke.yaml` |
| Silicon capture | ❌ none | simulation-derived only |

## Evidence

```bash
cargo test -p labwired-core --test pic24_ka_smoke
# test result: ok. 7 passed; 0 failed

cargo run -q -p labwired-cli -- test \
  --script examples/pic24f04ka200/io-smoke.yaml --no-uart-stdout
# PASS  2/2 checks · io-smoke · 400 steps
```

The committed ELF was rebuilt from `firmware/xc16-uart-smoke.c` with XC16
v1.70 and `p24F04KA200.gld`; its SHA-256 is
`f60fcdc3b76e24f658d01e18baca5dee9af0f30be63235f740d20bae783ae11d`.

Runtime facts: the XC16 startup vectors through the reset GOTO at 0x000000,
reaches `main()`, writes `H` and `I` to `U1TXREG` (0x0224), makes RA0 an
output and toggles `LATA0` until `max_steps`. The reset direction registers
are the package masks (TRISA 0x007F, TRISB 0xC310, TRISC 0).

## Scope and limitations

Validated: boot, UART1 TX, package GPIO masks and RA0. Not validated or not
modelled for this part: data EEPROM (absent on 04KA), UART2 (absent), timers,
ADC, SPI, I2C, RTCC, CTMU, comparators, deep sleep, PPS and alternate pin
functions. The public core's flash/SRAM backing is the larger
dsPIC33FJ32MC204-sized store; the descriptor still records the exact part
geometry (4 KB / 512 B), and the real image fits unchanged.

Sources: Microchip **DS30009937C**, *PIC24F04KA201 Family Data Sheet*
(Table 1-1, Tables 4-10/4-12/4-13, Figures 4-1/4-3, §4.2). Retrieved from
`https://ww1.microchip.com/downloads/en/DeviceDoc/30009937c.pdf`.
