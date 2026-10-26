# PIC24F16KA102 — LabWired example (L1)

Exact descriptor for the 28-pin **PIC24F16KA102** (DS39927C): 16 KB flash,
1536 B data SRAM, 512 B data EEPROM at 0x7FFE00, all PORTA<7:0> and
PORTB<15:0> pins (24 I/O pins, no PORTC). The public PIC24 core models the
package GPIO masks and UART1; this example boots a real XC16 v1.70 image that
prints `HI` on UART1 and then toggles RA0 forever.

| Item | Value |
| :--- | :--- |
| Chip | PIC24F16KA102, 28-pin SPDIP/SSOP/SOIC/QFN |
| Descriptor | `configs/chips/pic24f16ka102.yaml` |
| System | `system.yaml` (device-free; UART1 is CPU-internal) |
| Firmware | `firmware/pic24f16ka102-xc16-uart-smoke.elf` (real XC16 v1.70 build) |
| Smoke | `io-smoke.yaml` — expects `HI` on UART1, stops at `max_steps` |

## Run it

```bash
# from the repository root
cargo run -q -p labwired-cli -- test \
  --script examples/pic24f16ka102/io-smoke.yaml --no-uart-stdout
```

## Build the ELF

```bash
docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$PWD/examples/pic24f16ka102/firmware:/work" -w /work \
  fstlx/xc16:latest bash -lc '
    xc16-gcc -mcpu=24F16KA102 -O0 -Wall \
      -Wl,-T,/opt/microchip/xc16/support/PIC24F/gld/p24F16KA102.gld \
      -o pic24f16ka102-xc16-uart-smoke.elf xc16-uart-smoke.c'
```

## Scope and limitations

Modelled and validated: reset/boot from the XC16 vector, UART1 TX, the exact
28-pin PORTA/PORTB masks and the RA0 output path. The descriptor records the
part's real 16 KB / 1536 B geometry, while the public core's flash/SRAM
backing is the larger dsPIC33FJ32MC204-sized store (the real image fits
unchanged).

Not modelled or not exercised for this part: data EEPROM (512 B at
0x7FFE00), UART2 at 0x0230, timers, ADC, SPI, I2C, RTCC, CTMU, comparators,
deep sleep, PPS and the package's alternate pin functions. No silicon capture
exists, so every result here is simulation-derived. See `VALIDATION.md` for
the evidence trail and `REQUIRED_DOCS.md` for the primary sources.
