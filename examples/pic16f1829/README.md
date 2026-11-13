# PIC16F1829 — LabWired example

The Microchip [PIC16F1829](https://www.microchip.com/en-us/product/PIC16F1829)
(DS40001440E): an 8-bit PIC16 **enhanced mid-range** part with 8K x 14-bit
words of flash, 1024 B SRAM, 256 B data EEPROM, a 16-level hardware return
stack and a 32 MHz maximum Fosc, here in the 20-pin PDIP
(`PIC16F1829-I/P`). This is the **L1 slice** and the first enhanced
mid-range part. It runs on the PIC16 core (`crates/core/src/cpu/pic16.rs`),
which a device-table flag switches into enhanced mode: BSR banking, linear
and program-flash FSR access, the 49-instruction set, a 16-level stack with
STVREN, and hardware context save. The board is the Microchip **Curiosity
8-bit development board (DM164137)**, whose 20-pin socket takes the part.

## What runs here

`firmware/sdcc-uart-smoke.c`, compiled by **SDCC 4.4.0** (pic14 port,
enhanced target) and linked by **gputils 1.4.0** with the stock
`16f1829_g.lkr`:

- selects the 16 MHz HFINTOSC (`OSCCON = 0x7A`) and waits for
  `OSCSTAT.HFIOFR`,
- configures the EUSART for 9600 baud (`SPBRGL = 103`,
  `TXSTA = TXEN | BRGH`, `RCSTA = SPEN`) with TX on RB7,
- makes **RA5** (LED D4) an output, sets `LATA5` and prints what `RA5`
  reads back, then toggles it three times and checks it ended low,
- sums a stack buffer through a pointer (FSR + `MOVIW`),
- raises three software INT interrupts (`INTF`),
- prints a `switch` result (a computed-jump table),
- runs a checked work loop while **TMR0** (Fosc/4, no prescaler) overflows
  every 256 cycles. The ISR clobbers W, STATUS, BSR and PCLATH and does not
  save them, because SDCC relies on the enhanced core's hardware shadow
  registers. So if `RETFIE` restored the wrong context, main's results
  would come out wrong (with the shadow restore disabled the line reads
  `TMR0 IRQ BAD`),
- checks two 16-bit multiplies.

```text
PIC16F1829 OK
LED=1
TOGGLE OK
SUM OK
IRQ OK
three many
TMR0 IRQ OK
MUL OK
```

The image programs CONFIG1/CONFIG2 at words 0x8007/0x8008 (byte 0x1000E,
past the 64 KiB HEX boundary): INTOSC, WDT off, power-up timer on, MCLR on,
STVREN on, LVP off.

SDCC emits Intel HEX; `firmware/hex2elf.py` wraps it byte for byte in an
`EM_MCHP_PIC` ELF and the chip's `arch: pic16` selects the PIC16 core
(`arch_policy::image_arch_for_chip`). `crates/core/tests/pic16f1829_smoke.rs`
and `ci/test.sh` both check that the ELF payload equals the HEX.

## Build and run

```bash
# from the repository root
./target/release/labwired test \
  --script examples/pic16f1829/io-smoke.yaml \
  --output-dir out/pic16f1829-smoke --no-uart-stdout

./target/release/labwired \
  --firmware examples/pic16f1829/firmware/pic16f1829-sdcc-uart-smoke.elf \
  --system configs/systems/pic16f1829-curiosity.yaml --max-steps 200000
```

Rebuilding the firmware needs the upstream SDCC release (distribution
packages usually leave out the PIC ports) and gputils:

```bash
SDCC=/path/to/sdcc-4.4.0/bin/sdcc examples/pic16f1829/firmware/build.sh
```

`build.sh` passes `--use-non-free`: SDCC's pic14 device libraries (the
`pic16f1829` SFR definitions and startup code) live in its `non-free` tree
under Microchip's license, not the GPL, and they are linked into the
committed HEX/ELF.

The build is deterministic: HEX
`b2e273952e58952318f7fa3ae60ef1dfa39e5b1528401cd6794d03c9a1b7dfb1`, ELF
`26cf89c9e070b8738bcd0b99e5c56855aeedd02bd533dd044fcd632edee569f3`.

## Hardware

A Curiosity 8-bit board (DM164137) with a PIC16F1829 in the 20-pin socket:
LED D4 is on RA5, and the EUSART TX (RB7) is on the socket's expansion
header. Connect it to a 3.3 V/5 V USB-serial adapter to see the console. The
board's other parts (potentiometer, push button, mTouch pad, mikroBUS) are
not modelled. The part is sold by distributors as `PIC16F1829-I/P` (PDIP-20),
`-I/SO`, `-I/SS` and `-I/ML`.

See [VALIDATION.md](VALIDATION.md) for the evidence and the scope limits.
