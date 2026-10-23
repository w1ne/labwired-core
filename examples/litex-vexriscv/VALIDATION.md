# Validation — LiteX VexRiscv SoC

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x10000000` |
| VexRiscv core executing the committed firmware (standard RV32 profile) | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Integrated SRAM (0x10000000, 256 KiB) load + execute | smoke-tested | firmware links and runs there |
| LiteX UART PA-style polled console (`txfull` → `rxtx`) | smoke-tested | `LITEX VEXRISCV SMOKE` + heartbeat lines with `txfull=` readback |
| LiteX timer one-shot + zero event | smoke-tested | `timer=running` then `timer=zero` (the event only latches if the countdown reached zero) |
| Unmodelled CSR window (SPI-flash controller) answers zeros | smoke-tested | `spi=00000000` printed from 0x60005000 |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Timer zero-event IRQ delivery into the CPU | **not wired** | the status/pending registers are the firmware-visible surface; the engine does not deliver external IRQs to RISC-V cores yet |
| SPI flash controller / Ethernet MAC / MMCM | **not modelled** | declared `type: stub`; the SPI window is mapped memory, no controller behind it |
| Main RAM (DDR, 0x40000000, 256 MiB) | **not mapped** | the engine allocates memory regions densely; a 256 MiB window is not useful to the smoke |
| LiteX BIOS + flash-load boot flow | **not modelled** | fast boot loads the ELF and enters it directly |
| LEDs / switches / buttons | not modelled | no board_io bindings are declared (see `EXTERNAL_COMPONENTS.md`) |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED; no bitstream diff exists |

## Commands and evidence (verified 2026-10-20)

```bash
# from core/
rustup target add riscv32im-unknown-none-elf   # once
cargo build -p firmware-litex-vexriscv-smoke --release --target riscv32im-unknown-none-elf
#   Finished `release` profile — no warnings

./target/release/labwired test \
  --script examples/litex-vexriscv/io-smoke.yaml \
  --output-dir out/litex-vexriscv-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 1.28s

./target/release/labwired \
  --firmware target/riscv32im-unknown-none-elf/release/firmware-litex-vexriscv-smoke \
  --system configs/systems/litex-vexriscv.yaml --max-steps 400000
#   Initial PC: 0x10000000
#   LITEX VEXRISCV SMOKE
#   cpu=rv32im uart=litex
#   spi=00000000
#   timer=running
#   timer=zero
#   hb=0 txfull=00000000
#   …

# Determinism: two runs, identical streams
./target/release/labwired --firmware … --system … --max-steps 4000000 | grep -E '^(LITEX|cpu=|spi=|timer=|hb=)' > run1
./target/release/labwired --firmware … --system … --max-steps 4000000 | grep -E '^(LITEX|cpu=|spi=|timer=|hb=)' > run2
diff run1 run2   # clean

source "$HOME/.cargo/env"
./scripts/unsupported_instruction_audit.sh \
  --firmware target/riscv32im-unknown-none-elf/release/firmware-litex-vexriscv-smoke \
  --system configs/systems/litex-vexriscv.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/litex-vexriscv
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0
#   report: out/unsupported-audit/litex-vexriscv/report.md

cargo test -p labwired-core --lib litex_timer
#   6 passed — reset values, one-shot, periodic re-arm, W1C, IRQ gate,
#   update_value latch

cargo test -p labwired-core --test firmware_survival test_litex_vexriscv_smoke_survival
#   1 passed — the committed fixture boots and prints the banner
```

## Notes

- `timer=running` is `ev_status` reporting a non-zero countdown and
  `timer=zero` is `ev_pending` latching after the one-shot's 100,000 cycles;
  both lines disappear if the countdown or the event latch regress.
- The heartbeat's `txfull=` field is read back from the UART CSR after the
  TX-ready poll, so the assertion checks the register the firmware actually
  observed.
- The timer model advances from the bus's elapsed-cycle ticks, so it is
  independent of the tick interval (closed-form periodic re-arm).
- The RISC-V profile is selected from the descriptor (`core: rv32im` →
  standard RV32), not the ESP32-C3 default; the C3's own profile is unchanged.
