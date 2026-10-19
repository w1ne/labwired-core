# LiteX VexRiscv SoC — smoke example

Runs a bare-metal RV32IM firmware on the LiteX VexRiscv SoC: the firmware
boots from the SoC's integrated SRAM, prints over the LiteX UART, and its
heartbeat reports the UART `txfull` register it actually observed.

This is the first RISC-V soft-core target in the engine. The modelled
surface is deliberately small — CPU, memory map, and the LiteX UART console
path — and everything else is declared, not faked: unmodelled CSR windows
are `type: stub` and answer zeros, and the smoke prints one of those
readbacks (`timer=00000000`) so the omission is visible on the wire.

## Build

```bash
cd core
rustup target add riscv32im-unknown-none-elf   # once
cargo build -p firmware-litex-vexriscv-smoke --release --target riscv32im-unknown-none-elf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/riscv32im-unknown-none-elf/release/firmware-litex-vexriscv-smoke \
  --system examples/litex-vexriscv/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
LITEX VEXRISCV SMOKE
cpu=rv32im uart=litex
timer=00000000
hb=0 txfull=00000000
hb=1 txfull=00000000
...
```

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/litex-vexriscv/io-smoke.yaml \
  --output-dir out/litex-vexriscv-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| VexRiscv CPU (`rv32im`, standard RV32 profile) | modelled |
| Integrated SRAM (0x10000000, 256 KB) | modelled |
| SPI-flash window (0x20000000, 32 MB) | mapped (reads 0xFF untouched) |
| Integrated ROM window (0x0, 256 KB) | mapped; contents (LiteX BIOS) not modelled |
| LiteX UART (0x60001800) | modelled — polled console path |
| LiteX timer / SPI / Ethernet / MMCM CSRs | `type: stub` — zeros |
| Main RAM (0x40000000, 256 MB on the SoC) | not mapped — see `VALIDATION.md` |
| LEDs / switches / buttons | not modelled — see `EXTERNAL_COMPONENTS.md` |

## Boot path

Fast boot: the engine loads the ELF and enters it at its entry point with SP
seeded at the top of SRAM. The real board boots the LiteX BIOS out of the
integrated ROM, which then loads an application image from SPI flash; that
BIOS and the flash load are not modelled, and this example does not claim
them.

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what a LiteX board carries beyond this SoC
- `VALIDATION.md` — claims, evidence, and explicit non-claims
