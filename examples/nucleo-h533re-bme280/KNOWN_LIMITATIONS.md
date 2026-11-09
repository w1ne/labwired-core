# Known limitations — NUCLEO-H533RE with BME280

- The focused H533 profile models SRAM1 only; SRAM2, SRAM3, backup SRAM, and
  TrustZone attribution are outside the profile.
- Only GPIOA/PA5, GPIOB/PB6/PB7, I2C1, RCC, and SysTick are exposed for this
  smoke. The board button, VCP, USB, and other peripherals are omitted.
- The BME280 model returns deterministic environmental-register values. The
  smoke checks its chip ID and I2C transaction path; it does not establish
  physical sensor accuracy or analog/electrical behavior.
- The simulator assumes an ideal supply rail; power sequencing and brown-out
  behavior are not simulated.
- Evidence is model-derived only, not hardware-confirmed.
