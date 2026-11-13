# PIC16F877A — LabWired example

The Microchip [PIC16F877A](https://www.microchip.com/en-us/product/PIC16F877A)
(DS39582B): an 8-bit PIC16 classic mid-range part with 8K x 14-bit words of
flash, 368 B SRAM, 256 B data EEPROM, an 8-level hardware return stack and a
20 MHz maximum Fosc, here in the 40-pin PDIP (`PIC16F877A-I/P`). This is the
**L1 slice** and the first part on the new public PIC16 core
(`crates/core/src/cpu/pic16.rs`): boot, USART TX and the PORTB LED latch,
from real C firmware.

## What runs here

`firmware/sdcc-uart-smoke.c`, compiled by **SDCC 4.4.0** (pic14 port) and
linked by **gputils 1.4.0** with the stock `16f877a_g.lkr`:

- configures the USART for 9600 baud at Fosc = 16 MHz (`SPBRG = 25`,
  `TXSTA = TXEN | BRGH`, `RCSTA = SPEN`) with RC6 as an output,
- makes **RB0** an output, sets it and prints the value it reads back,
- toggles RB0 three times and checks it ended low.

```text
PIC16F877A OK
LED=1
TOGGLE OK
```

On the way the SDCC C runtime exercises the core, not a hand fixture: bank
switching through `STATUS.RP0` (TRIS/TXSTA/SPBRG live in bank 1), locals in
the 0x70-0x7F block every bank shares, and generic-pointer string reads,
which SDCC implements as `MOVWF PCLATH` / `MOVWF PCL` computed jumps into
`RETLW` tables. The image also programs the configuration word at word
0x2007 (HS oscillator, WDT off, power-up timer on, LVP off).

SDCC emits Intel HEX; `firmware/hex2elf.py` wraps it byte for byte in an
`EM_MCHP_PIC` ELF. Both 8-bit PIC families share that ELF machine value, so
the loader reports the image as PIC18 and the chip's `arch: pic16` selects
the PIC16 core (`arch_policy::image_arch_for_chip`).
`crates/core/tests/pic16f877a_smoke.rs` and `ci/test.sh` both check that the
ELF payload equals the HEX.

## Build and run

```bash
# from the repository root
./target/release/labwired test \
  --script examples/pic16f877a/io-smoke.yaml \
  --output-dir out/pic16f877a-smoke --no-uart-stdout

./target/release/labwired \
  --firmware examples/pic16f877a/firmware/pic16f877a-sdcc-uart-smoke.elf \
  --system configs/systems/pic16f877a-demo.yaml --max-steps 20000
```

Rebuilding the firmware needs the upstream SDCC release (distribution
packages usually leave out the PIC ports) and gputils:

```bash
SDCC=/path/to/sdcc-4.4.0/bin/sdcc examples/pic16f877a/firmware/build.sh
```

`build.sh` passes `--use-non-free`: SDCC's pic14 device libraries (the
`pic16f877a` SFR definitions and startup code) live in its `non-free` tree
under Microchip's license, not the GPL, and they are linked into the
committed HEX/ELF.

The build is deterministic: HEX
`1c156f52ddc9487cbb293be46ac82819b8f23be7a1e31bb68437f34ca2b4b7df`, ELF
`8b54696664e3da4a0620ae1a726feab43bb68db5bda1fcda68afb44581d17932`.

## Hardware

Any PIC16F877A board with a 16 MHz crystal, the USART TX (RC6) to a
USB-serial adapter or RS-232 driver, and an LED on RB0. No board-specific
peripherals are modelled. The part is sold by distributors as
`PIC16F877A-I/P` (PDIP-40) and `PIC16F877A-I/PT` (TQFP-44).

See [VALIDATION.md](VALIDATION.md) for the evidence and the scope limits.
