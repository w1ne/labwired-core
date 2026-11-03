# Validation — STM32F407G-DISC1 (STM32F407VGT6)

**Status: local simulator onboarding smoke passed 2026-11-03.** The scripted
run passed all 4 assertions at 4,000,000 steps using the
`stm32f407vg` chip descriptor and `configs/systems/stm32f4-discovery.yaml`.
The committed run is under `evidence/`; this is not a physical-board test.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| ELF boot and Cortex-M4F execution | smoke-tested | `labwired test` passed 4/4 assertions at 4,000,000 steps |
| RCC gates for GPIOA/GPIOD and USART2 | smoke-tested | firmware bring-up and UART output |
| User LEDs PD12–PD15 | smoke-tested | UART reports each LED's output-register state; test asserts at least four LD4/PD12 GPIO edges |
| USART2 PA2 TX from 16 MHz HSI | smoke-tested | smoke banner and heartbeat lines captured |
| USER button PA0 input | declared only | system manifest maps it; firmware does not read it |
| Supply behavior | ideal rail assumed | no routed power net; power-on and brown-out reset are not simulated |
| Other chip peripherals | not covered by this smoke | no behavior claim is made for them here |
| Onboard sensors/audio/USB/ST-LINK | not modeled as board devices | see `EXTERNAL_COMPONENTS.md` |

## Commands and evidence

Tool: local `labwired-cli`

Firmware target: `thumbv7em-none-eabi`

Firmware SHA-256: `fdf70f5b0e761a25fbb939d1a82c45dd9c64714f7b042dfbae6c00786170ba50`

System chip: `configs/chips/stm32f407vg.yaml`

```sh
cargo build -p firmware-f4-discovery-smoke --release --target thumbv7em-none-eabi
scripts/onboarding_smoke.sh \
  --target-id stm32f4-discovery \
  --crate firmware-f4-discovery-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f4-discovery/io-smoke.yaml \
  --system configs/systems/stm32f4-discovery.yaml \
  --out-dir examples/stm32f4-discovery/evidence
```

The committed result records **4/4 assertions passing** and the UART output
shows `F4-DISCO SMOKE` plus on/off readbacks for LD4/PD12, LD3/PD13, LD5/PD14,
and LD6/PD15. Detailed build and run output is under `evidence/`.

The heartbeat's `odr=` value is read back from `GPIOD_ODR` after each write.
The board's ST-LINK VCP is not connected to the MCU UART by default (UM1472);
the firmware uses PA2/PA3 on header P1. The simulator captures USART2 output
directly and does not model ST-LINK or physical UART wiring. The result also
records that VDD is assumed to be an ideal 3.3 V rail, so power-on and
brown-out reset behavior are outside this smoke.
