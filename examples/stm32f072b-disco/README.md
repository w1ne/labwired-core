# STM32F072B-DISCO — smoke example

Runs a bare-metal Cortex-M0 firmware on the STM32F072B-DISCO: the firmware
enables the F0 clock gates, configures USART1 on PA9/PA10, walks the four user
LEDs with `GPIOC_ODR` readback, reads the pulled-up user button and heartbeats.

## Build

```bash
cd core
rustup target add thumbv6m-none-eabi   # once
cargo build -p firmware-stm32f072b-disco-smoke --release --target thumbv6m-none-eabi
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv6m-none-eabi/release/firmware-stm32f072b-disco-smoke \
  --system examples/stm32f072b-disco/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
STM32F072B-DISCO SMOKE
cpu=cortex-m0 uart=usart1
led=ld3 odr=00000040
led=ld4 odr=00000140
led=ld5 odr=00000340
led=ld6 odr=000003c0
btn=1
hb=0 odr=000003c0 btn=1
...
```

The `odr=` field is the GPIOC output register read from the model (LD3 = PC6 =
bit 6, LD4 = PC8 = bit 8, LD5 = PC9 = bit 9, LD6 = PC7 = bit 7); `btn=1` is the
released button on pulled-up PA0.

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/stm32f072b-disco/io-smoke.yaml \
  --output-dir out/stm32f072b-disco-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| STM32F072RBT6 CPU (`cortex-m0`, 48 MHz) | modelled |
| Flash (0x08000000, 128 KiB) / SRAM (0x20000000, 16 KiB) | modelled |
| RCC (F0 layout: CR/CFGR/APBxENR/CFGR2/CFGR3/CR2) | modelled |
| GPIOA–F (v2) + AHBENR gates | modelled |
| USART1–4 (v2) + clock gates | modelled — USART1 output is captured by the simulator; ST-LINK/V2 is SWD-only |
| SPI1/2 (legacy), I2C1/2 (legacy), TIM1/2/3/14/15/16/17 | modelled |
| DMA1, EXTI, SYSCFG, PWR, CRC, bxCAN1, DAC1, RTC, IWDG/WWDG | modelled |
| ADC1 | modelled — `stm32l4`-family profile (not exercised) |
| USB FS, TSC, CRS | `type: stub` — zeros |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources this package is grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
