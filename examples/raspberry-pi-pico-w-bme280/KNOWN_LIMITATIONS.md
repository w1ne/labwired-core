# Raspberry Pi Pico W + BME280 profile limits

- The descriptor represents only RP2040 startup, UART0, IO_BANK0, SIO, I2C0,
  and the attached BME280 model used by the smoke.
- CYW43439 wireless behavior, the wireless-driven onboard LED, USB/UF2 boot,
  and power sequencing are not modeled here.
- The sensor is attached at I2C address `0x77`; the profile does not discover a
  different address at runtime.
- The simulator does not establish physical supply compatibility, bus pull-up
  values, analog edge quality, or hardware timing margins.
- No physical Pico W or BME280 capture is included. The evidence is from the
  repository's deterministic simulation runner.
