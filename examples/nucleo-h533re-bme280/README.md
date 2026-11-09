# NUCLEO-H533RE with BME280

The demand-ranked onboarding list places STM32H5 under **other STM32, rank 4
(13%)**; Nucleo and Discovery boards are used in **23%** of surveyed projects.
This board profile attaches an Adafruit BME280 breakout to the NUCLEO-H533RE's
Arduino I2C1 pins and keeps the existing LD2 indicator. The firmware reads the
BME280 chip ID (`0x60`) and pulses LD2 only when the read succeeds.

## Reproduce

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h533re-bme280 \
  --crate firmware-nucleo-h533re-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-h533re-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h533re-bme280.yaml \
  --out-dir examples/nucleo-h533re-bme280/evidence
```

The evidence directory contains the CLI and firmware build logs, simulation
result, and onboarding metrics. Vendor lifecycle and live availability checks
are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Board mapping and scope

ST UM3121 maps Arduino D15/SCL to PB6 and D14/SDA to PB7. I2C1 and GPIOB are
real H5 models; the BME280 is an existing I2C model at address `0x77`. The
focused chip profile also exposes RCC, GPIOA, SysTick, flash, and SRAM1 for
these smokes. The profile does not claim the H533's other SRAM banks, VCP,
button, USB, TrustZone, or unlisted peripherals. This is simulator evidence,
not a physical-board test.

Sources: [ST UM3121 board manual](https://www.st.com/resource/en/user_manual/dm00941891.pdf),
[STM32H533RE product status](https://www.st.com/en/microcontrollers-microprocessors/stm32h533re),
[NUCLEO-H533RE product page](https://estore.st.com/en/nucleo-h533re-cpn.html), and
[Adafruit BME280 PID 2652](https://www.adafruit.com/product/2652).
