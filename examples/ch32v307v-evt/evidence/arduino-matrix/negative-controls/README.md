# Negative controls — ch32v307 Arduino matrix row

Each is the passing row's firmware with one thing taken away; each must fail.
The system/chip files were throwaway copies (`<neg>/...`) of the committed ones
with only the named change.

| Control | Change | Result |
|---|---|---|
| `L0_serial_boot-nopllword` | chip without `pll_cfg_word` (0x1FFFF70C) | memory violation at 0x1ffff70c, step 966; no UART |
| `L3_i2c_sensor-ina219-on-i2c1` | INA219 attached to I2C1 instead of I2C2 | `LW_L3_FAIL nack` — Wire really drives I2C2 |
| `L6_pwm-watch-PA1` | `pwm_watch: gpioa:1` instead of `gpioa:3` | oracle_fail — the edges are on PA3 only |
| `L8_can-cangate` | bxCAN1 clock gate moved to APB1PCENR bit 26 | `LW_L8_FAIL` — CAN1 needs CAN1EN (bit 25) |
