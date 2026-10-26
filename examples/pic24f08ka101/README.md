# PIC24F08KA101 — LabWired example (L1)

Exact descriptor for the 20-pin **PIC24F08KA101** (DS39927C): 8 KB flash,
1536 B data SRAM, 512 B data EEPROM at 0x7FFE00, PORTA<6:0> and
PORTB<15:12, 9:7, 4, 2:0> only (18 I/O pins, no PORTC). The public PIC24
core models the package GPIO masks and UART1; this example boots a real XC16
v1.70 image that prints `HI` on UART1 and then toggles RA0 forever.

| Item | Value |
| :--- | :--- |
| Chip | PIC24F08KA101, 20-pin PDIP/SSOP/SOIC/QFN |
| Descriptor | `configs/chips/pic24f08ka101.yaml` |
| System | `system.yaml` (device-free; UART1 is CPU-internal) |
| Firmware | `firmware/pic24f08ka101-xc16-uart-smoke.elf` (real XC16 v1.70 build) |
| Smoke | `io-smoke.yaml` — expects `HI` on UART1, stops at `max_steps` |

## Run it

```bash
# from the repository root
cargo run -q -p labwired-cli -- test \
  --script examples/pic24f08ka101/io-smoke.yaml --no-uart-stdout
```

## Build the ELF

```bash
docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$PWD/examples/pic24f08ka101/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=24F08KA101 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/PIC24F/gld/p24F08KA101.gld \
      -o pic24f08ka101-xc16-uart-smoke.elf xc16-uart-smoke.c'
```

## Scope and limitations

Modelled and validated: reset/boot from the XC16 vector, UART1 TX, the exact
20-pin PORTA/PORTB masks and the RA0 output path. The descriptor records the
part's real 8 KB / 1536 B geometry, while the public core's flash/SRAM
backing is the larger dsPIC33FJ32MC204-sized store (the real image fits
unchanged).

Not modelled or not exercised for this part: data EEPROM (512 B at
0x7FFE00), UART2 at 0x0230, timers, ADC, SPI, I2C, RTCC, CTMU, comparators,
deep sleep, PPS and the package's alternate pin functions. No silicon capture
exists, so every result here is simulation-derived. See `VALIDATION.md` for
the evidence trail and `REQUIRED_DOCS.md` for the primary sources.
