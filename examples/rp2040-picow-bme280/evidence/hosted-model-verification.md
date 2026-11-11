# Hosted LabWired model verification

Checked 2026-11-11 against the shared RP2040 sensor path using LabWired's
generic `rpi-pico` catalog target. The catalog does not expose a distinct Pico W
board model, so this does not verify Pico W's CYW43439 radio, WL_GPIO0 LED, USB,
or physical board wiring.

- Diagram: `rpi-pico` MCU plus the modeled Adafruit 2652 BME280 component at
  I2C address `0x77`; GP4/GP5 wired to SDA/SCL.
- Firmware: Arduino `Wire` read of BME280 register `0xD0`, then serial marker.
- Oracle: serial contains `PICOW_BME280_OK ID=0x60 I2C0=GP4/GP5`.
- Result: `status: model_verified`, `run_status: pass`,
  `stop_reason: assertions_passed`.
- Reported non-blocking approximations: device time uses nominal `cpu_hz`
  without tracking PLL reconfiguration; VDD is assumed ideal because no supply
  rail is routed.

This is digital-twin evidence, not hardware confirmation. The local bare-metal
smoke and its full run logs are recorded alongside this report.
