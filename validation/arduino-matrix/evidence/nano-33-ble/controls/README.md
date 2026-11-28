# Controls for the `nano-33-ble` matrix row (2026-11-20)

Each control runs one cell's firmware and test script on the row's system with
one thing removed (`system.yaml` here). Every control must fail.

| Control | Cell | Removed | Result |
| --- | --- | --- | --- |
| `no-ina219` | L3_i2c_sensor | the INA219 on Wire | FAIL, prints `LW_L3_FAIL nack` |
| `no-max31855` | L4_spi_sensor | the MAX31855 on SPI | FAIL, prints `LW_L4_FAIL frame=0x00000000` |
| `no-adc-input` | L5_adc | the 1650 mV on A0 (`input_mv`) | FAIL, prints `LW_L5_VAL 0 0` |
| `no-host` | L0_serial_boot | the USB host (`host: true`) | FAIL, no serial output |
| `no-bootloader-uart` | L0_serial_boot | `bootloader_rx_running` on UARTE0 | FAIL, no serial output (the core waits for `RXTO`) |
| `no-vector-offset` | L0_serial_boot | `memory_overrides.reset_vector_offset` | FAIL at step 0, memory access violation |

`run.log` is the CLI output and `uart.log` the captured serial text. The
firmware is the matrix build of the cell (`<out>/nano-33-ble/<cell>/firmware.elf`).

L6_pwm has no control and no pad oracle on this row: see `../../../PROBLEMS.md`.
