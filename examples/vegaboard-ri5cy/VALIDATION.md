# Validation — Vega board (RI5CY)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x0` (flash base) |
| RI5CY RV32IMC core executing the committed firmware (standard RV32 profile) | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps; `mul=0000002a` from a real multiply |
| Flash (0x0, 1 MiB) + SRAM (0x20000000, 192 KiB) load + execute | smoke-tested | firmware links into flash and runs with the stack in SRAM |
| NXP LPUART0 polled console (STAT TDRE → DATA) | smoke-tested | `VEGABOARD RI5CY SMOKE` + heartbeat with `stat=` readback |
| Unmodelled SCG/PCC windows answer zeros | smoke-tested | `scg=00000000 pcc0=00000000` printed from 0x4002C010/0x4002B000 |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| SCG / PCC clock blocks | **not modelled** | declared `type: stub`; the smoke polls, no clock gating is modelled |
| INTMUX interrupt controller | **not modelled** | declared `type: stub` |
| Low-power timers (LPTMR0/1/2) | **not modelled** | declared `type: stub` |
| Board-level user I/O / sensors / radio | **not modelled** | the platform defines no GPIO bank |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (verified 2026-10-23)

```bash
# from core/
cargo build -p firmware-vegaboard-ri5cy-smoke --release --target riscv32imc-unknown-none-elf
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/vegaboard-ri5cy/io-smoke.yaml \
  --output-dir out/vegaboard-ri5cy-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.75s

./target/release/labwired \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-vegaboard-ri5cy-smoke \
  --system configs/systems/vegaboard-ri5cy.yaml --max-steps 400000
#   Initial PC: 0x0
#   VEGABOARD RI5CY SMOKE / cpu=rv32imc uart=lpuart
#   mul=0000002a / scg=00000000 pcc0=00000000 / hb=0 stat=00c00000 …

# Determinism: two runs, identical streams
… | grep -E '^(VEGABOARD|cpu=|mul=|scg=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-vegaboard-ri5cy-smoke \
  --system configs/systems/vegaboard-ri5cy.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/vegaboard-ri5cy
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --test firmware_survival test_vegaboard_ri5cy_smoke_survival
```

## Notes

- `stat=00c00000` is read back from the LPUART status register (TDRE|TC
  idle), not assumed.
- The chip declares `core: rv32imc`; the smoke is built for `riscv32imc`, so
  the toolchain enforces the ISA it uses.
- The platform declares no core clock; the descriptor models 50 MHz and the
  smoke does not depend on it (the UART is polled).
