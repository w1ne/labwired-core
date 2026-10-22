# LiteX Minerva SoC — smoke example

Runs a bare-metal RV32I firmware on the LiteX Minerva SoC: the firmware boots
from the SoC's integrated SRAM, proves the RV32I core with an add/shift-only
result, prints the declared-stub timer readback, and prints over the LiteX
UART.

This is the RISC-V soft-core sibling of the `litex-vexriscv` target, on the
RV32I Minerva core (no multiply/divide, no compressed instructions — the
firmware is built for `riscv32i-unknown-none-elf`, so the toolchain enforces
the ISA).

## Build

```bash
cd core
rustup target add riscv32i-unknown-none-elf   # once
cargo build -p firmware-litex-minerva-smoke --release --target riscv32i-unknown-none-elf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/riscv32i-unknown-none-elf/release/firmware-litex-minerva-smoke \
  --system examples/litex-minerva/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
MINERVA LITEX SMOKE
cpu=rv32i uart=litex
acc=00000100
timer=00000000
hb=0 txfull=00000000
...
```

`acc` is 1 doubled eight times (add/shift only); `timer=00000000` is the
declared-stub timer window (this SoC uses the 8-bit-CSR variant, not the
modelled 32-bit CSR timer).

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/litex-minerva/io-smoke.yaml \
  --output-dir out/litex-minerva-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| Minerva CPU (`rv32i`, standard RV32 profile) | modelled |
| Integrated SRAM (0x10000000, 256 KB) | modelled |
| Integrated memory (0x0, 256 KB) | mapped as a memory window |
| ROM (0x20020000, 32 KB) | mapped; contents (LiteX BIOS) not modelled |
| Boot flash (0x20228000, 1 MB) | declared as `flash` (reads 0xFF untouched) |
| LiteX UART (0xE0001800) | modelled — polled console path |
| LiteX timer, 8-bit CSR variant (0xE0002800) | `type: stub` — zeros |
| LiteX Ethernet MAC (0xE0003800) | `type: stub` |
| Main RAM (0x40000000, 256 MB on the SoC) | not mapped |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Boot path

Fast boot: the engine loads the ELF and enters it at its entry point with SP
seeded at the top of SRAM. The real board boots the LiteX BIOS from the ROM,
which then loads an application image from boot flash; that BIOS and the
flash load are not modelled.

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what the platform carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
