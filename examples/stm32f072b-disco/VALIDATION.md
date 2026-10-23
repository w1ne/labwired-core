# Validation — STM32F072B-DISCO

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the entry point (PC init) | smoke-tested | run below: `Initial PC: 0x08000000` (flash base) |
| Cortex-M0 executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Flash (0x08000000, 128 KiB) + SRAM (0x20000000, 16 KiB) load + execute | smoke-tested | firmware links into flash and runs with the stack in SRAM |
| F0 RCC clock gates (AHBENR GPIOA/GPIOC, APB2ENR USART1) | smoke-tested | LED/USART accesses work only after the firmware sets the gates; the model drops unclocked accesses |
| USART1 console (PA9/PA10, AF1; BRR 0x45 at 8 MHz) | smoke-tested | banner + heartbeat on the TX sink |
| Four-LED GPIO walk with ODR readback | smoke-tested | `led=ld3 odr=00000040` … `led=ld6 odr=000003c0` |
| Pulled-up user button readback | smoke-tested | `btn=1` (released) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical UART streams (`diff` clean) |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| USB FS, TSC, CRS | **not modelled** | declared `type: stub` |
| Joystick / touch sensor | **not modelled** | not declared |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED |

## Commands and evidence (verified 2026-10-23)

```bash
# from core/
cargo build -p firmware-stm32f072b-disco-smoke --release --target thumbv6m-none-eabi
#   Finished `release` profile — warning-free

./target/release/labwired test \
  --script examples/stm32f072b-disco/io-smoke.yaml \
  --output-dir out/stm32f072b-disco-smoke --no-uart-stdout
#   PASS  3/3 checks · io-smoke · 4000000 steps · 0.13s

./target/release/labwired \
  --firmware target/thumbv6m-none-eabi/release/firmware-stm32f072b-disco-smoke \
  --system configs/systems/stm32f072b-disco.yaml --max-steps 400000
#   Initial PC: 0x08000000
#   STM32F072B-DISCO SMOKE / cpu=cortex-m0 uart=usart1
#   led=ld3 odr=00000040 … led=ld6 odr=000003c0 / btn=1 / hb=0 …

# Determinism: two runs, identical streams
… | grep -E '^(STM32|cpu=|led=|btn=|hb=)' > run1
… > run2 ; diff run1 run2   # clean

./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv6m-none-eabi/release/firmware-stm32f072b-disco-smoke \
  --system configs/systems/stm32f072b-disco.yaml \
  --max-steps 200000 --out-dir out/unsupported-audit/stm32f072b-disco
#   simulator exit 0 · Instructions executed: 200000 · Unsupported: 0

cargo test -p labwired-core --test firmware_survival test_stm32f072b_disco_smoke_survival
```

## Notes

- The GPIOC `odr=` values are read back from the model, not assumed: LD3 = PC6
  (bit 6), LD4 = PC8 (bit 8), LD5 = PC9 (bit 9), LD6 = PC7 (bit 7).
- Only the reset HSI (8 MHz) is used; the F0 PLL and CFGR3 kernel-clock muxing
  are modelled but not exercised by this smoke.
- The ADC uses the `stm32l4`-family profile (the CFGR1/CFGR2 block); it is
  declared but not exercised. USB/TSC/CRS are stubs.
