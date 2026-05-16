# STM32F401CDU6 Black Pill Validation Runbook

Run all commands from `core/`.

## Physical Scope

Physical validation is ST-Link USB/SWD sanity only. This onboarding does not connect a physical UART, so UART `OK` output is accepted only as simulation evidence from LabWired trace artifacts.

## Modeled Chip Scope

The descriptor covers the STM32F401xB/xC peripheral memory map used by STM32F401CDU6. LabWired uses executable models for RCC, GPIO, SysTick, USART1/2/6, I2C1/2/3, SPI1/2/3/4, TIM1/2/3/4/5/9/10/11, ADC1, and EXTI. Blocks that are present in the chip map but do not yet have dedicated F401-compatible behavior are declared as stubs: DMA1/2 stream controllers, RTC, WWDG, IWDG, I2S extension windows, PWR, SDIO, SYSCFG, CRC, FLASH control, USB OTG FS windows, and DBG. `ADC_Common` is intentionally not declared as a separate window because it is nested inside the ADC1 address range and the current descriptor validator rejects overlapping peripheral regions. Shared timer IRQ lines are also limited by the descriptor schema's single-IRQ field; TIM2/3/4/5 have dedicated IRQs, while TIM1/TIM9/TIM10/TIM11 are modeled for register access and ticking without claiming full shared-IRQ fidelity.

## 1) Optional: ensure target installed

```bash
rustup target add thumbv7em-none-eabi
```

## 2) Validate manifests

```bash
cargo run -q -p labwired-cli -- asset validate --chip configs/chips/onboarding/stm32f401cdu6-blackpill.yaml
cargo run -q -p labwired-cli -- asset validate --system configs/systems/onboarding/stm32f401cdu6-blackpill.yaml
```

Pass criteria:
1. both commands exit `0`

## 3) Build smoke firmware

```bash
cargo build -p firmware-f401cdu6-blackpill-demo --release --target thumbv7em-none-eabi
```

## 4) Run deterministic trace smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f401cdu6-blackpill/trace-smoke.yaml \
  --output-dir out/stm32f401cdu6-blackpill/trace-smoke \
  --no-uart-stdout \
  --trace \
  --trace-max 128
```

Pass criteria:
1. exit code is `0`
2. `out/stm32f401cdu6-blackpill/trace-smoke/uart.log` contains `OK`
3. `result.json`, `snapshot.json`, and `trace.json` exist
4. stop reason is `max_steps`

## 5) Run deterministic I2C smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f401cdu6-blackpill/i2c-smoke.yaml \
  --output-dir out/stm32f401cdu6-blackpill/i2c-smoke \
  --no-uart-stdout \
  --trace \
  --trace-max 512
```

Pass criteria:
1. exit code is `0`
2. `out/stm32f401cdu6-blackpill/i2c-smoke/uart.log` contains `I2C_OK`
3. `result.json`, `snapshot.json`, and `trace.json` exist
4. stop reason is `max_steps`

## 6) Run direct JSON/VCD simulation

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-f401cdu6-blackpill-demo \
  --system configs/systems/stm32f401cdu6-blackpill.yaml \
  --max-steps 32 \
  --json \
  --vcd out/stm32f401cdu6-blackpill/blackpill.vcd
```

Pass criteria:
1. JSON reports `status: "finished"`
2. VCD exists at `out/stm32f401cdu6-blackpill/blackpill.vcd`

## 7) Confirm ST-Link USB/SWD sanity

```bash
lsusb
st-info --probe
```

Pass criteria:
1. output includes `0483:3748 STMicroelectronics ST-LINK/V2`
2. `st-info --probe` reports one ST-Link programmer and an F4-class target

Observed local hardware evidence: the ST-Link probe was visible and
`st-info --probe` reported an F4-class target with 384KB flash and 96KB SRAM,
matching the STM32F401CDU6 simulation profile. Physical validation remains
probe/SWD sanity only because no physical UART is connected.
