# RISC-V `virt` machine — smoke example

Runs a bare-metal RV32IMC firmware on the generic RISC-V `virt` machine: the
firmware loads into the flat RAM window at 0x80000000, proves the core with a
real multiply, reads back the declared-stub PLIC/CLINT windows, and prints
over the NS16550 UART at 0x10000000.

## Build

```bash
cd core
rustup target add riscv32imc-unknown-none-elf   # once
cargo build -p firmware-riscv-virt-smoke --release --target riscv32imc-unknown-none-elf
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-riscv-virt-smoke \
  --system examples/riscv-virt/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
RISCV VIRT SMOKE
cpu=rv32imac uart=ns16550
mul=0000002a
plic=00000000 clint=00000000
hb=0 lsr=60
...
```

`lsr=60` is the NS16550 line-status register (THRE|TEMT idle), read back from
the CSR; `plic`/`clint` are the declared-stub windows.

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/riscv-virt/io-smoke.yaml \
  --output-dir out/riscv-virt-smoke --no-uart-stdout
```

## What the machine models

| Surface | State |
| :--- | :--- |
| RV32IMAC CPU (standard RV32 profile) | modelled |
| RAM code window (0x80000000, 1 MiB) | modelled as `flash` |
| RAM data/stack window (0x80100000, 7 MiB) | modelled as `ram` |
| NS16550 UART0 (0x10000000) | modelled — THR/LSR polled console path |
| CLINT (0x02000000) | `type: stub` — zeros |
| PLIC (0x0C000000) | `type: stub` — zeros |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources the descriptor and this package are grounded on
- `EXTERNAL_COMPONENTS.md` — what the machine carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
