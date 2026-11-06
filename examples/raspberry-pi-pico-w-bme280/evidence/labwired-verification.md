# Hosted LabWired verification

- `labwired_validate`: 0 errors; 2 `I2C_NO_PULLUP` warnings because the
  Playground diagram does not include discrete pull-up parts. The repository
  system smoke models register-level I2C transactions; it does not model bus
  analog behavior.
- `labwired_verify`: **`status: model_verified`** at a 30,000,000-step budget.
  Both declared serial clauses passed: `BME280_ID=0x60` and
  `PICO_W_BME280_SMOKE_OK`.
- The hosted check used an Arduino `Wire` sketch on the catalog `rpi-pico` board
  with the verified Adafruit 2652 BME280 module selection. The committed
  repository smoke uses the separate Rust firmware crate and also asserts raw
  temperature bytes `0x082DC0`.
- The verifier reported two non-blocking approximations: device time is derived
  from the configured CPU frequency without tracking PLL changes, and VDD is
  assumed to be an ideal rail because no routed power net is present. This is
  model evidence, not physical-board confirmation.
