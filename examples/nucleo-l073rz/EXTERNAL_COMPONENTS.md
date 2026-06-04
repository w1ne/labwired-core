# External components — NUCLEO-L073RZ

The smoke/onboarding demo uses **no external components**. Everything it
touches is on-chip or on the Nucleo carrier board:

| Component | Where | Role in the demo |
|-----------|-------|------------------|
| LD2 (green LED) | on-board, PA5 | toggled by the demo (BSRR) |
| B1 (user button) | on-board, PC13 | declared in `board_io`, not driven by the demo |
| ST-LINK/V2-1 VCP | on-board debug MCU | carries USART2 TX bytes to the host |

`external_devices: []` in the system manifest reflects this — no sensors,
displays, or bus peripherals are wired.

## Adding external devices later

To grow this into a sensor lab (cf. `examples/adxl345-sensor-lab`,
`examples/bme280-weather-lab`), wire a device to one of the on-chip buses
and add it under `external_devices` in the system manifest. Candidate buses
already present in `configs/chips/stm32l073.yaml`:

- **I2C1** (`0x40005400`) — SCL/SDA on PB8/PB9 or PB6/PB7 (check UM1724 CN
  headers).
- **SPI1** (`0x40013000`) — SCK/MISO/MOSI on PA5/PA6/PA7 (PA5 is shared with
  LD2; pick SPI2 or remap if the LED is also used).
- **USART1** (`0x40013800`) — spare UART for a second serial device.

Note the I2C/SPI/ADC register models are reused from the L4 family and are
not L0-tuned (see `VALIDATION.md`), so a new external-device lab should
re-validate the relevant bus before being marked silicon-accurate.
