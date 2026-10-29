# Validation — NUCLEO-L552ZE-Q (STM32L552ZET6Q)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x8000800, SP: 0x20030000` |
| Cortex-M33 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| L5 RCC clock gates (`AHB2ENR` GPIO, `APB1ENR2.LPUART1EN`) | smoke-tested | firmware bring-up + LPUART output |
| GPIO PC7/PB7/PA9 drive + ODR readback | smoke-tested | `led=ld1 odr=00000080` … asserted in `io-smoke.yaml` |
| LPUART1 PG7 TX @ 115200 from 4 MHz MSI | smoke-tested | `NUCLEO-L552ZE-Q SMOKE` + heartbeat lines on the console sink |
| PC13 button input | smoke-tested (reads 0 released) | `btn=0` line |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| TrustZone / SAU / GTZC / secure aliasing | **not modelled** | core stores MPU registers but does not enforce them |
| MPU / PMSAv8 / `TT`/`TTA` | **not modelled** | `docs/isa_coverage.md` known gap |
| L5 non-secure FLASH controller, EXTI, ADC, FDCAN, USB | declared stubs | `configs/chips/stm32l552.yaml` |
| SPI/I2C/timers/DMA/RNG/RTC | modelled, **unexercised** | declared; no firmware path in this slice |
| USB / Ethernet / radio | **not modelled** | — |

## Commands and evidence (verified 2026-10-29)

```bash
# from the repository root
cargo build -p firmware-l552ze-smoke --release --target thumbv8m.main-none-eabi
#   Finished `release` profile — no warnings

./target/release/labwired test \
  --script examples/nucleo-l552ze-q/io-smoke.yaml \
  --output-dir out/nucleo-l552ze-q-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.81s

./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml --max-steps 400000
#   Initial PC: 0x8000800, SP: 0x20030000
#   NUCLEO-L552ZE-Q SMOKE
#   cpu=cortex-m33 uart=lpuart1
#   led=ld1 odr=00000080
#   ...
#   hb=0 odr=00000080 btn=0

# Determinism: two runs, identical UART + snapshot
./target/release/labwired test --script ... --output-dir out/run1 --no-uart-stdout
./target/release/labwired test --script ... --output-dir out/run2 --no-uart-stdout
cmp out/run1/uart.log out/run2/uart.log        # clean
cmp out/run1/snapshot.json out/run2/snapshot.json  # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/nucleo-l552ze-q
#   Instructions executed: 200000 · Unsupported: 0
```

## Notes

- The heartbeat's `odr=` field is read back from the port's `ODR` after each
  write, so the assertions check the pad the firmware actually drove.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The `stm32l5` RCC profile is reference-manual/CMSIS derived
  (`stm32l552xx.h`), not silicon-verified: there is no L552 bench part.
- The board's real LEDs are LD1 PC7, LD2 PB7, LD3 PA9 (UM2623 and the Zephyr
  board dts). Some test platform fixtures wire a stand-in green LED to PA5
  (Arduino D13); that is not this board's LD1.
