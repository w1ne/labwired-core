# Validation — Digilent Arty A7-35T (LiteX VexRiscv SoC)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x10000000` |
| VexRiscv core executing the committed firmware (standard RV32 profile) | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Integrated SRAM (0x10000000, 256 KiB) load + execute | smoke-tested | firmware links and runs there |
| LiteX UART polled console (`txfull` → `rxtx`) | smoke-tested | `ARTY LITEX SMOKE` + heartbeat with `txfull=` readback |
| LiteX timer one-shot + zero event | smoke-tested | `timer=running` then `timer=zero` |
| CAS `Leds` drive + readback (LD4–LD7) | smoke-tested | `leds=00000001 … 0000000f … 00000000` read back from the CSR |
| CAS `Switches` / `ButtonsStatus` levels | smoke-tested | `sw=0000000f btn=0000000f` (released active-low inputs read high) |
| CAS `ButtonsPending` W1C + `ButtonsEnabled` | smoke-tested | `pend=00000000 en=0000000f` after the clear/enable sequence |
| board_io bindings (4 LEDs, 4 switches, 4 buttons) | wired | attached at bus build; the button attach proves the driven level with `read_gpio_input` |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Button press → pending/IRQ path under stimulus | **not exercised by the CLI smoke** | modelled and unit-tested (`active_low` press edge latches; `ButtonsEnabled` gates the level); a session-driven stimulus is a follow-up |
| SPI-flash controller / Ethernet MAC / MMCM | **not modelled** | declared `type: stub` |
| Main RAM (DDR, 0x40000000, 256 MiB) | **not mapped** | dense allocation; not needed by the smoke |
| LiteX BIOS + flash-load boot flow | **not modelled** | fast boot loads the ELF directly |
| Silicon / bitstream capture | **none** | SIM-DERIVED; no executing-fidelity differential |

## Commands and evidence (verified 2026-10-22)

```bash
# from core/
cargo build -p firmware-arty-litex-smoke --release --target riscv32im-unknown-none-elf
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/arty-litex-vexriscv/io-smoke.yaml \
  --output-dir out/arty-litex-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 2.39s

./target/release/labwired \
  --firmware target/riscv32im-unknown-none-elf/release/firmware-arty-litex-smoke \
  --system configs/systems/arty-litex-vexriscv.yaml --max-steps 400000
#   Initial PC: 0x10000000
#   ARTY LITEX SMOKE / cpu=rv32im cas=litex
#   leds=00000001 / 00000002 / 00000004 / 00000008 / 0000000f / 00000000
#   sw=0000000f btn=0000000f
#   pend=00000000 en=0000000f
#   timer=running → timer=zero
#   hb=0 txfull=00000000 …

# Determinism: two runs, identical streams
… | grep -E '^(ARTY|cpu=|leds=|sw=|pend=|timer=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/riscv32im-unknown-none-elf/release/firmware-arty-litex-smoke \
  --system configs/systems/arty-litex-vexriscv.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/arty-litex-vexriscv
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --lib litex_control_status     # 6 passed
cargo test -p labwired-core --test firmware_survival test_arty_litex_smoke_survival
```

## Notes

- Each `leds=` line is the value **read back** from the CAS `Leds` register
  after the write, so the assertion checks the CSR, not the firmware's intent.
- The released switch/button lines read high because the board wiring is
  active-low; `buttons_active_low: true` makes a press (line low) latch
  `ButtonsPending`.
- The CAS model implements the `PinPort` and board capability hooks, so a
  session or the browser can drive the switches/buttons and read the LEDs.
