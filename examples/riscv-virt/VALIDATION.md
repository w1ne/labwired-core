# Validation — RISC-V `virt` machine

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x80000000` |
| RV32IMAC core executing the committed firmware (standard RV32 profile) | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps; `mul=0000002a` from a real multiply |
| Flat RAM (0x80000000) load + execute | smoke-tested | firmware links into the 1 MiB code window and runs |
| NS16550 UART0 polled console (LSR THRE → THR) | smoke-tested | `RISCV VIRT SMOKE` + heartbeat with `lsr=` readback |
| Unmodelled PLIC/CLINT windows answer zeros | smoke-tested | `plic=00000000 clint=00000000` printed from 0x0C000000/0x02000000 |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| PLIC (interrupt delivery) | **not modelled** | declared `type: stub`; firmware polls |
| CLINT MMIO | **not modelled** | declared `type: stub`; the `time` CSR still advances |
| Virtio / PCI / RTC / GPIO | **not modelled** | not part of the modelled machine |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (verified 2026-10-23)

```bash
# from core/
cargo build -p firmware-riscv-virt-smoke --release --target riscv32imc-unknown-none-elf
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/riscv-virt/io-smoke.yaml \
  --output-dir out/riscv-virt-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.68s

./target/release/labwired \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-riscv-virt-smoke \
  --system configs/systems/riscv-virt.yaml --max-steps 400000
#   Initial PC: 0x80000000
#   RISCV VIRT SMOKE / cpu=rv32imac uart=ns16550
#   mul=0000002a / plic=00000000 clint=00000000 / hb=0 lsr=60 …

# Determinism: two runs, identical streams
… | grep -E '^(RISCV|cpu=|mul=|plic=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-riscv-virt-smoke \
  --system configs/systems/riscv-virt.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/riscv-virt
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --test firmware_survival test_riscv_virt_smoke_survival
```

## Notes

- The chip declares `core: rv32imac`; the smoke is built for `riscv32imc`
  (the A extension is present on the core but the single-hart smoke needs no
  atomics), so the toolchain enforces the ISA it uses.
- `lsr=60` is read back from the UART's line-status register (THRE|TEMT idle),
  not assumed.
- The machine's RAM is modelled as an 8 MiB window (a machine parameter on the
  real platform); the code window is the low 1 MiB.
