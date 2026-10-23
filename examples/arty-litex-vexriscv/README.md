# Digilent Arty A7-35T — LiteX VexRiscv SoC smoke example

Runs a bare-metal RV32IM firmware on the Arty A7-35T's LiteX VexRiscv SoC:
the firmware boots from the SoC's integrated SRAM, walks the four user LEDs
through the Control-and-Status (CAS) register with a readback at every step,
prints the switch/button levels and the button pending/enable registers, runs
the LiteX timer as a one-shot, and prints over the LiteX UART.

This is the board-level companion to the `litex-vexriscv` SoC target. The
modelled surface is deliberately small — CPU, memory map, LiteX UART, LiteX
timer and the CAS block — and everything else is declared, not faked:
unmodelled CSR windows are `type: stub` and answer zeros.

## Build

```bash
cd core
rustup target add riscv32im-unknown-none-elf   # once
cargo build -p firmware-arty-litex-smoke --release --target riscv32im-unknown-none-elf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/riscv32im-unknown-none-elf/release/firmware-arty-litex-smoke \
  --system examples/arty-litex-vexriscv/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
ARTY LITEX SMOKE
cpu=rv32im cas=litex
leds=00000001
leds=00000002
leds=00000004
leds=00000008
leds=0000000f
leds=00000000
sw=0000000f btn=0000000f
pend=00000000 en=0000000f
timer=running
timer=zero
hb=0 txfull=00000000
...
```

With no buttons pressed, the active-low switch/button lines read high
(`0x0f`); the board_io bindings let a session or the browser drive them.

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/arty-litex-vexriscv/io-smoke.yaml \
  --output-dir out/arty-litex-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| VexRiscv CPU (`rv32im`, standard RV32 profile) | modelled |
| Integrated SRAM (0x10000000, 256 KB) | modelled |
| SPI-flash window (0x20000000, 32 MB) | mapped (reads 0xFF untouched) |
| Integrated ROM window (0x0, 256 KB) | mapped; contents (LiteX BIOS) not modelled |
| LiteX UART (0x60001800) | modelled — polled console path |
| LiteX timer (0x60002800) | modelled — countdown, `update_value` latch, zero event |
| Control-and-Status (0x60006800) | modelled — `Leds` R/W, `Switches`, `ButtonsStatus`, `ButtonsPending` (W1C), `ButtonsEnabled` |
| Board I/O: LD4–LD7 (cas 0–3), SW0–SW3 (32–35), BTN0–BTN3 (64–67) | modelled via the CAS + `board_io` |
| LiteX SPI / Ethernet / MMCM CSRs | `type: stub` — zeros |
| Main RAM (0x40000000, 256 MB on the SoC) | not mapped |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Boot path

Fast boot: the engine loads the ELF and enters it at its entry point with SP
seeded at the top of SRAM. The real board boots the LiteX BIOS out of the
integrated ROM, which then loads an application image from SPI flash; that
BIOS and the flash load are not modelled.

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
