# Vega board (RI5CY) — smoke example

Runs a bare-metal RV32IMC firmware on the PULP Vega board's RI5CY SoC: the
firmware boots from flash at 0x0, proves the core with a real multiply, reads
back the declared-stub SCG/PCC windows, and prints over LPUART0 at 0x40042000.

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
scg=00000000 pcc0=00000000
hb=0 stat=00c00000
...
```

`stat=00c00000` is the LPUART status register (TDRE|TC idle), read back from
the CSR; `scg`/`pcc0` are the declared-stub clock windows.

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
| RI5CY CPU (`rv32imc`, standard RV32 profile) | modelled |
| Flash (0x0, 1 MiB) | modelled |
| SRAM (0x20000000, 192 KiB) | modelled |
| LPUART0 (0x40042000) | modelled — LPUART profile, polled console path |
| LPUART1 (0x40043000) | modelled |
| SCG CSR / PCC0 / PCC1 | `type: stub` — zeros |
| INTMUX, LPTMR0/1/2 | `type: stub` |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
