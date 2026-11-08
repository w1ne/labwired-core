# Negative controls — ch32x035 Arduino matrix row

Each is the passing row's firmware with one thing taken away; each must fail.
The system/chip files were throwaway copies (`<neg>/...`) of the committed ones
with only the named change.

| Control | Change | Result |
|---|---|---|
| `L0_serial_boot-no-pad-map` | chip without `pad_map: ch32x03x` on USART1 | no UART (captured len=0): with the F1 table, TX is PA9, and PB10 never opens the console gate |
| `L3_i2c_sensor-no-ina219` | system without the INA219 | `LW_L3_FAIL nack`: Wire really addresses the device on I2C1 |
| `L6_pwm-watch-PA7` | `pwm_watch: gpioa:7` instead of `gpioa:6` | oracle_fail: the UART says `LW_L6_OK`, but the edges are on PA6 only |
