# STM32F4-Discovery — LabWired example

The [STM32F4DISCOVERY](https://www.st.com/en/evaluation-tools/stm32f4discovery.html)
board (MB997, rev. B/C) with an **STM32F407VGT6**: Cortex-M4F, 1 MB flash,
128 KB main SRAM (+64 KB CCM), 168 MHz max. This package is the promoted,
runnable path for the board; the staged Renode-derived descriptor remains in
`configs/chips/onboarding/` as inventory only.

## What runs here

A bare-register smoke firmware (`crates/firmware-f4-discovery-smoke`) that:

- runs a light over the four user LEDs **PD12..PD15** (LD4/LD3/LD5/LD6),
- prints a heartbeat on **USART2** (PA2 TX / PA3 RX) at 115200 8N1 from the
  16 MHz HSI,
- reports in each line the state **read back from `GPIOD_ODR`**, so a passing
  run proves the pad actually changed level.

On this board the on-board ST-LINK VCP is **not wired** to the MCU UART
(UM1472): the console pins are read from the PA2/PA3 header pins.

The reusable [analog fixture](analog-ntc-diode-system.yaml) routes a generic
Beta-model NTC divider and an unrelated Shockley diode load line to PA1/ADC1_IN1
and PA4/ADC1_IN4. PA0 remains mapped to the on-board USER button, and PA2/PA3
remain the USART2 console pins. The board-specific co-simulation test solves
both netlists and checks the resulting firmware-visible ADC counts. The
[PJF gate-emission fixture](analog-pjfet-gate-emission-adc.yaml) extends that
route to PA5/PA6 and checks the generic level-1 PJF gate-diode emission
coefficient through the same ADC.

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

The on-board LIS302DL/LIS3DSH accelerometer, CS43L22 audio DAC, MP45DT02
microphone, USB OTG, Ethernet, CAN, SDIO and the ST-LINK bridge are **not
modelled**. See `VALIDATION.md` for the tier table.
