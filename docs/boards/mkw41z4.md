# NXP KW41Z (MKW41Z512VHT4)

Cortex-M0+ BLE 4.2 + IEEE 802.15.4 wireless MCU. This profile covers the core
and the sensor/peripheral path (UART, GPIO, clock, I2C/SPI, timers); the radio
blocks (BTLE_RF / GENFSK / ZLL / XCVR) exist in the vendored SVD but are
intentionally not modeled, per the header comment in
`configs/chips/mkw41z4.yaml`.

## Status at a glance

| Aspect      | Status                                                                      |
|-------------|------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/mkw41z4.yaml`](../../configs/chips/mkw41z4.yaml)             |
| Validation  | `firmware_survival::kw41z_smoke`, `kw41z_nxp`, `kw41z_zephyr`, `kw41z_zephyr_fxos8700`, `kw41z_lcd_activity`; register-level twin `tests/kw41z_clock_boot.rs` |
| Tier        | **sim-validated** — deep peripheral models + green sim tests, no silicon diff |

## What is proven

- Bare-metal smoke firmware and vendor-NXP-HAL clock/UART bring-up
  (`kw41z_smoke`, `kw41z_nxp`)
- UNMODIFIED upstream Zephyr v3.7 `hello_world` for board `frdm_kw41z`
  (`kw41z_zephyr`), through the real MCG/RSIM clock bring-up path
- Zephyr FXOS8700 I2C sensor read + Nokia5110/PCD8544 LCD activity-bar demo
  end to end (`kw41z_zephyr_fxos8700`, `kw41z_lcd_activity`)
- Kinetis LPUART (DATA/STAT/CTRL), MCG, RSIM, I2C1, SPI0, and GPIOC are
  modeled with real register behavior; PORTA/B/C, GPIOA/B, I2C0, TPM0, PIT,
  SIM, PMC, SMC, RCM, and TRNG0 are declarative/stub entries
- ADC0 (Kinetis ADC16, the model shared with the FRDM-K64F): generic NTC and
  diode netlists routed to PTB2/PTB3 and PTB18/PTA19 convert through
  `analog_pins` at 12 and 16 bits, continuous conversion follows the NTC, and
  `AIEN` pends ADC0_IRQn 15 (`tests/cosim_frdm_kw41z_adc16_analog.rs`)

## What is NOT proven

- No KW41Z silicon diff — no bench part.
- BLE and 802.15.4 radio (BTLE_RF/GENFSK/ZLL/XCVR) are not modeled.
- ADC0 hardware triggers, temperature/bandgap channels, conversion timing,
  SIM_SCGC6 clock gating, and the VALTH rail (left at the model's 1.2 V
  default) are not modeled or asserted.

## Sources

- Register surface ingested from the public CMSIS-SVD
  (`cmsis-svd-data: data/NXP/MKW41Z4.svd`) via `svd-ingestor`; see
  `configs/peripherals/mkw41z4/`.
- ADC0 base/IRQ/layout: mcux-sdk `devices/MKW41Z4/MKW41Z4.h`; ADC pads: Zephyr
  hal_nxp @cc9b44ae3e `dts/nxp/kinetis/kwx/pinctrl/MKW41Z512CAT4-pinctrl.h`.
