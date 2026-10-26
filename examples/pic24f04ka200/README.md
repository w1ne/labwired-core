# PIC24F04KA200 — LabWired example (L1)

Exact descriptor for the 14-pin **PIC24F04KA200** (DS30009937C): 4 KB flash,
512 B data SRAM, no data EEPROM, PORTA<6:0> and PORTB<15:14, 9:8, 4> only
(12 I/O pins, no PORTC). The public PIC24 core models the package GPIO masks
and UART1; this example boots a real XC16 v1.70 image that prints `HI` on
UART1 and then toggles RA0 forever.

| Item | Value |
| :--- | :--- |
| Chip | PIC24F04KA200, 14-pin PDIP/TSSOP |
| Descriptor | `configs/chips/pic24f04ka200.yaml` |
| System | `system.yaml` (device-free; UART1 is CPU-internal) |
| Firmware | `firmware/pic24f04ka200-xc16-uart-smoke.elf` (real XC16 v1.70 build) |
| Smoke | `io-smoke.yaml` — expects `HI` on UART1, stops at `max_steps` |

## Run it

```bash
# from the repository root
cargo run -q -p labwired-cli -- test \
  --script examples/pic24f04ka200/io-smoke.yaml --no-uart-stdout
```

## Build the ELF

```bash
docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$PWD/examples/pic24f04ka200/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=24F04KA200 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/PIC24F/gld/p24F04KA200.gld \
      -o pic24f04ka200-xc16-uart-smoke.elf xc16-uart-smoke.c'
```

## Scope and limitations

Modelled and validated: reset/boot from the XC16 vector, UART1 TX, the exact
14-pin PORTA/PORTB masks and the RA0 output path. The descriptor records the
part's real 4 KB / 512 B geometry, while the public core's flash/SRAM backing
is the larger dsPIC33FJ32MC204-sized store (the real image fits unchanged).

Not modelled or not exercised for this part: data EEPROM (absent on 04KA),
UART2 (also absent), timers, ADC, SPI, I2C, RTCC, CTMU, comparators, deep
sleep, PPS and the package's alternate pin functions. No silicon capture
exists, so every result here is simulation-derived. See `VALIDATION.md` for
the evidence trail and `REQUIRED_DOCS.md` for the primary sources.
