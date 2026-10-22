# Validation — LiteX Minerva SoC

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x10000000` |
| Minerva RV32I core executing the committed firmware (standard RV32 profile) | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps; `acc=00000100` from an add/shift-only loop |
| Integrated SRAM (0x10000000, 256 KiB) load + execute | smoke-tested | firmware links and runs there |
| LiteX UART polled console (`txfull` → `rxtx`) | smoke-tested | `MINERVA LITEX SMOKE` + heartbeat with `txfull=` readback |
| Unmodelled timer window answers zeros | smoke-tested | `timer=00000000` printed from 0xE0002800 |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| LiteX timer (8-bit CSR variant) | **not modelled** | declared `type: stub`; the 32-bit CSR timer model is a different register map |
| Ethernet MAC | **not modelled** | declared `type: stub` |
| Main RAM (DDR, 0x40000000) | **not mapped** | dense allocation; not needed by the smoke |
| LiteX BIOS + boot-flash load | **not modelled** | fast boot loads the ELF directly |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (verified 2026-10-22)

```bash
# from core/
cargo build -p firmware-litex-minerva-smoke --release --target riscv32i-unknown-none-elf
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/litex-minerva/io-smoke.yaml \
  --output-dir out/litex-minerva-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.83s

./target/release/labwired \
  --firmware target/riscv32i-unknown-none-elf/release/firmware-litex-minerva-smoke \
  --system configs/systems/litex-minerva.yaml --max-steps 400000
#   Initial PC: 0x10000000
#   MINERVA LITEX SMOKE / cpu=rv32i uart=litex
#   acc=00000100 / timer=00000000 / hb=0 txfull=00000000 …

# Determinism: two runs, identical streams
… | grep -E '^(MINERVA|cpu=|acc=|timer=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/riscv32i-unknown-none-elf/release/firmware-litex-minerva-smoke \
  --system configs/systems/litex-minerva.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/litex-minerva
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --test firmware_survival test_litex_minerva_smoke_survival
```

## Notes

- The firmware is built for RV32I, so the toolchain enforces the core's ISA:
  a multiply or divide would fail the build rather than silently execute on a
  model that happens to support it.
- `timer=00000000` documents the declared stub; this platform uses the
  8-bit-CSR `LiteX_Timer` variant, whose register map differs from the 32-bit
  CSR timer modelled for the VexRiscv SoC.
- The chip declares `core: rv32i`, so runs select the standard RV32 profile,
  not the ESP32-C3 default.
