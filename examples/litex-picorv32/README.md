# LiteX PicoRV32 SoC — smoke example

Runs a bare-metal RV32IMC firmware on the LiteX PicoRV32 SoC: the firmware
boots from the SoC's integrated SRAM, proves the RV32IMC core with a real
multiply, prints the declared-stub timer readback, and prints over the LiteX
UART.

This is the third RISC-V soft-core target in the engine, on the PicoRV32 core
(the M and C extensions are present — the firmware is built for
`riscv32imc-unknown-none-elf`).

## Build

```bash
cd core
rustup target add riscv32imc-unknown-none-elf   # once
cargo build -p firmware-litex-picorv32-smoke --release --target riscv32imc-unknown-none-elf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-litex-picorv32-smoke \
  --system examples/litex-picorv32/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
PICORV32 LITEX SMOKE
cpu=rv32imc uart=litex
mul=0000002a
timer=00000000
hb=0 txfull=00000000
...
```

`mul=0000002a` is 7 × 6 through the M extension; `timer=00000000` is the
declared-stub timer window (this SoC uses the 8-bit-CSR variant at
0xE0002000).

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/litex-picorv32/io-smoke.yaml \
  --output-dir out/litex-picorv32-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| PicoRV32 CPU (`rv32imc`, standard RV32 profile) | modelled |
| Integrated SRAM (0x10000000, 256 KB) | modelled |
| Integrated memory (0x0, 256 KB) | mapped as a memory window |
| SPI-flash window (0x20000000, 32 MB) | declared as `flash` (reads 0xFF untouched) |
| LiteX UART (0x60001800) | modelled — polled console path |
| LiteX timer, 8-bit CSR variant (0xE0002000) | `type: stub` — zeros |
| LiteX SPI-flash controller (0x60005000) | `type: stub` |
| LiteX Ethernet MAC (0x60007800) | `type: stub` |
| Main RAM (0x40000000, 256 MB on the SoC) | not mapped |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Boot path

Fast boot: the engine loads the ELF and enters it at its entry point with SP
seeded at the top of SRAM. The real board boots the LiteX BIOS from the
integrated memory and loads an application from SPI flash; that BIOS and the
flash load are not modelled.

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what the platform carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
