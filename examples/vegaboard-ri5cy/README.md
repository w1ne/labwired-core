# Vega board (RI5CY) — smoke example

Runs a bare-metal RV32IMC firmware against the imported Renode Vega RI5CY
system: the firmware boots from flash at 0x0, proves the core with a real
multiply, and prints over modeled LPUART0 at 0x40042000. The system descriptor
is narrowed to these implemented surfaces and omits unmodeled clocks/timers.

## Build

```bash
cd core
rustup target add riscv32imc-unknown-none-elf   # once
cargo build -p firmware-vegaboard-ri5cy-smoke --release --target riscv32imc-unknown-none-elf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-vegaboard-ri5cy-smoke \
  --system examples/vegaboard-ri5cy/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
VEGABOARD RI5CY SMOKE
cpu=rv32imc uart=lpuart
mul=0000002a
hb=0 stat=00c00000
...
```

`stat=00c00000` is the LPUART status register (TDRE|TC idle), read back from
the modeled CSR.

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/vegaboard-ri5cy/io-smoke.yaml \
  --output-dir out/vegaboard-ri5cy-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| RI5CY execution profile (`rv32imc`) | modelled |
| Flash (0x0, 1 MiB) | modelled |
| SRAM (0x20000000, 192 KiB) | modelled |
| LPUART0 (0x40042000) | modelled — LPUART profile, polled console path |
| Clock generators, additional UARTs, timers and interrupt mux | omitted from this narrowed descriptor |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
