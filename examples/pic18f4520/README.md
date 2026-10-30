# PIC18F4520 — LabWired example

The Microchip [PIC18F4520](https://www.microchip.com/en-us/product/PIC18F4520)
(DS39631E): an 8-bit PIC18 with 32 KB flash, 1536 B SRAM, a 31-level hardware
return stack and a 40 MHz maximum Fosc, here in the 40-pin PDIP
(`PIC18F4520-I/P`). This is the **L1 slice** and the first PIC18 part on the
public PIC18 core: boot, EUSART TX and the PORTB LED latch, from real
C firmware.

## What runs here

`firmware/sdcc-uart-smoke.c`, compiled by **SDCC 4.4.0** (pic16 port) and
linked by **gputils 1.4.0** with the stock `18f4520_g.lkr`:

- configures the EUSART for 9600 baud at Fosc = 16 MHz (`SPBRG = 25`,
  `TXSTA = TXEN | BRGH`, `RCSTA = SPEN`) with RC6 as an output,
- makes **RB0** an output, sets `LATB0` and prints the latch it reads back,
- toggles `LATB0` three times and checks it ended low.

```text
PIC18F4520 OK
LED=1
TOGGLE OK
```

On the way the SDCC C runtime exercises the core, not a hand fixture: its
FSR1/FSR2 software stack (`MOVFF` through `POSTDEC1`/`PREINC1`/`PLUSW2`),
generic-pointer string reads from flash (`TBLRD*+`), and `MULLW`/`RLNCF`/
`DCFSNZ`-style pointer arithmetic. The image also programs the CONFIG words
at `0x300000` (HS oscillator, WDT off, LVP off, XINST off).

SDCC emits Intel HEX; `firmware/hex2elf.py` wraps it byte for byte in an
`EM_MCHP_PIC` ELF (the machine value XC8 uses), which is what LabWired loads.
`crates/core/tests/pic18f4520_smoke.rs` and `ci/test.sh` both check that the
ELF payload equals the HEX.

## Build and run

```bash
# from the repository root
./target/release/labwired test \
  --script examples/pic18f4520/io-smoke.yaml \
  --output-dir out/pic18f4520-smoke --no-uart-stdout

./target/release/labwired \
  --firmware examples/pic18f4520/firmware/pic18f4520-sdcc-uart-smoke.elf \
  --system configs/systems/pic18f4520-demo.yaml --max-steps 20000
```

Rebuilding the firmware needs the upstream SDCC release (distribution
packages usually leave out the pic16 port) and gputils:

```bash
SDCC=/path/to/sdcc-4.4.0/bin/sdcc examples/pic18f4520/firmware/build.sh
```

The build is deterministic: HEX
`85aefef9420c177d799d5cab1c623e45cd2bd2001317afb17f7c4bbb3755ab6f`, ELF
`dd272de1acd0df45ed25108ac5eeedc85c7f955ceb62b52b1cb4650651aac6e8`.

## Hardware

Any PIC18F4520 board with a 16 MHz crystal, the EUSART TX (RC6) to a
USB-serial adapter or RS-232 driver, and an LED on RB0. No board-specific
peripherals are modelled. The part is sold by distributors as
`PIC18F4520-I/P` (PDIP-40) and `PIC18F4520-I/PT` (TQFP-44).

See [VALIDATION.md](VALIDATION.md) for the evidence and the scope limits.
