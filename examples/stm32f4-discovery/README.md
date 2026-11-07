# STM32F407G-DISC1 (STM32F4-Discovery) — LabWired example

The current [STM32F407G-DISC1](https://www.st.com/en/evaluation-tools/stm32f4discovery.html)
order code (formerly STM32F4DISCOVERY) uses an **STM32F407VGT6**: Cortex-M4F,
1 MB flash, 128 KB main SRAM (+64 KB CCM), 168 MHz max. ST states that the
current order code replaces the original one. The LabWired system id remains
`stm32f4-discovery` for compatibility. This package is the promoted, runnable
path for the board; the staged Renode-derived descriptor remains in
`configs/chips/onboarding/` as inventory only.

## What runs here

A bare-register smoke firmware (`crates/firmware-f4-discovery-smoke`) that:

- runs a light over the four user LEDs **PD12..PD15** (LD4/LD3/LD5/LD6),
- prints a heartbeat on **USART2** (PA2 TX / PA3 RX) at 115200 8N1 from the
  16 MHz HSI,
- reports each LED's output state from `GPIOD_ODR` in the heartbeat. The smoke
  asserts UART output and GPIO edges for LD4 through the modeled board I/O.

On this board the on-board ST-LINK VCP is **not wired** to the MCU UART
(UM1472): the console pins are read from the PA2/PA3 header pins.

## Build and run

```bash
# from core/
cargo build -p firmware-f4-discovery-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f4-discovery/io-smoke.yaml \
  --output-dir out/f4-discovery-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-f4-discovery-smoke \
  --system configs/systems/stm32f4-discovery.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
F4-DISCO SMOKE
hb=0 ld4=on odr=00001000
hb=1 ld4=off odr=00000000
hb=2 ld3=on odr=00002000
hb=3 ld3=off odr=00000000
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | STM32F407VGT6, LQFP100 | UM1472 §2 |
| Core / clock | Cortex-M4F, 168 MHz max (HSI 16 MHz at boot) | DS8626 §2; RM0090 §6 |
| Flash / RAM | 1 MB @ 0x08000000 / 128 KB @ 0x20000000 (+64 KB CCM) | DS8626 Table 4 |
| User LEDs | LD4 PD12, LD3 PD13, LD5 PD14, LD6 PD15, active high | UM1472 §6.4; Zephyr `stm32f4_disco` |
| USER button | PA0, active high when pressed | UM1472 §6.3; MB997 button circuit |
| Console UART | USART2 PA2 (TX) / PA3 (RX), 115200 8N1 | Zephyr `stm32f4_disco` serial section |

## Explicitly not claimed

The on-board accelerometer, CS43L22 audio DAC, MP45DT02 microphone, USB OTG,
and ST-LINK/V2-A interface are **not modelled**. The STM32F407 chip descriptor
also declares peripherals beyond this smoke, but Ethernet, CAN, SDIO, I²C,
timers and ADC are not exercised here. See `VALIDATION.md` for the tier table.

See [`ONBOARDING.md`](ONBOARDING.md) and
[`evidence/vendor-status.md`](evidence/vendor-status.md) for the reproducible
smoke and the live stock check for the current order code.
