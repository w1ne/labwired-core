# Validation — STM32F4-Discovery (STM32F407VGT6)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x8000400, SP: 0x20020000` |
| Cortex-M4F core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| RCC clock gates (AHB1ENR GPIOA/GPIOD, APB1ENR USART2) | smoke-tested | firmware bring-up + UART output |
| GPIOD PD12..PD15 drive + ODR readback | smoke-tested | `hb=0 ld4=on odr=00001000` … asserted in `io-smoke.yaml` |
| USART2 PA2 TX @ 115200 from 16 MHz HSI | smoke-tested | `F4-DISCO SMOKE` + heartbeat lines on the console sink |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| USER button PA0 input | declared only | wired in the system manifest `board_io`; no firmware path reads it |
| I²C/SPI/timers/ADC/DMA/USB/Ethernet/CAN | **not modelled here** | no claims; see `EXTERNAL_COMPONENTS.md` |

## Commands and evidence (verified 2026-10-19)

```bash
# from core/
cargo build -p firmware-f4-discovery-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./target/release/labwired test \
  --script examples/stm32f4-discovery/io-smoke.yaml \
  --output-dir out/f4-discovery-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 3.09s

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-f4-discovery-smoke \
  --system configs/systems/stm32f4-discovery.yaml --max-steps 400000
#   Initial PC: 0x8000400, SP: 0x20020000
#   F4-DISCO SMOKE
#   hb=0 ld4=on odr=00001000
#   hb=1 ld4=off odr=00000000
#   hb=2 ld3=on odr=00002000
#   hb=3 ld3=off odr=00000000

# Determinism: two runs, identical streams
./target/release/labwired --firmware … --system … --max-steps 400000 | grep '^hb=' > run1
./target/release/labwired --firmware … --system … --max-steps 400000 | grep '^hb=' > run2
diff run1 run2   # clean

source "$HOME/.cargo/env"
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-f4-discovery-smoke \
  --system configs/systems/stm32f4-discovery.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/stm32f4-discovery
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0
#   report: out/unsupported-audit/stm32f4-discovery/report.md
```

## Notes

- The heartbeat's `odr=` field is read back from `GPIOD_ODR` after each write,
  so the assertions check the pad the firmware actually drove, not what it
  intended to drive.
- The board's ST-LINK VCP is not wired to the MCU UART (UM1467); the console
  pins are PA2/PA3. The example does not claim a USB console path.
