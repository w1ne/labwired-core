# STM32F103C8T6 Blue Pill + BME280

This example adds a buyable I²C sensor to the popular STM32F103C8T6 Blue Pill.
The JetBrains State of Embedded 2025 survey ranks STM32F103 **3rd** by MCU use
(17%; target-list item 4). The Blue Pill smoke reads the Bosch BME280's chip-ID
register over I²C1 and toggles the board's active-low PC13 LED.

## Wiring

| BME280 STEMMA QT breakout | Blue Pill |
| --- | --- |
| VIN / VCC | 3V3 |
| GND | GND |
| SCL | PB6 / I2C1_SCL |
| SDA | PB7 / I2C1_SDA |

The breakout includes I2C pull-ups and defaults to address `0x77` (SDO high).
For a bare BME280 sensor instead of the breakout, add pull-ups on SDA/SCL and
tie SDO to VDD for `0x77` or GND for `0x76`.

To observe the firmware UART on physical hardware, connect a 3.3 V USB-to-UART
adapter's RX to PA9 / USART1_TX and join grounds. The simulation captures
USART1 directly.

## Build and run

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103c8-bluepill-bme280 \
  --crate firmware-stm32f103c8-bluepill-bme280-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103c8-bluepill-bme280/io-smoke.yaml \
  --system configs/systems/stm32f103c8-bluepill-bme280.yaml \
  --out-dir examples/stm32f103c8-bluepill-bme280/evidence
```

The expected UART marker is `BLUEPILL_BME280_ID=0x60`. The same smoke also
requires four PC13 LED transitions and the expected simulator stop condition.

## What the profile covers

The board profile includes only RCC, GPIOA/B/C, USART1, I2C1, and SysTick.
I2C1 is a modeled STM32F1 peripheral and the BME280 is attached at the bus
peripheral. The test does not model the physical cable, pull-ups, power rail,
or USB-to-UART adapter. SPI, timers, ADC, CAN, USB, DMA, I2C2, and the other
USARTs are outside this profile and are not claimed to work here.

See [`VALIDATION.md`](VALIDATION.md) for the run result and
[`evidence/vendor-status.md`](evidence/vendor-status.md) for the live stock and
lifecycle checks. The separate [`evidence/labwired-verification.md`](evidence/labwired-verification.md)
records the hosted BME280 model check and its limits.
