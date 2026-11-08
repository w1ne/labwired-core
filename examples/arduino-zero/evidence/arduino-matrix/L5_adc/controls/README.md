# L5 controls (Arduino Zero, stock analogRead on A0 = PA02 = AIN0)

The matrix cell stops 1000 steps after `LW_L5_OK`, which cuts the value line
short in `../uart.log`. These two runs use the same ELF with a 200000-step
settle so the whole line is captured.

| run | A0 drive (system `input_mv`) | UART |
|---|---|---|
| `a0_1650mV_full_line.uart.log` | 1650 mV (the matrix system) | `LW_L5_OK`, `LW_L5_VAL 512 512` |
| `a0_0mV_negative.uart.log` | 0 mV (system copy with `input_mv: { 0: 0 }`) | `LW_L5_FAIL`, `LW_L5_VAL 0 0` |

512 = 1650 mV x GAIN DIV2 x 1024 / INTVCC1 (3300 mV / 2), the AR_DEFAULT path
of ArduinoCore-samd. The negative run shows the sketch's exact-code check
does fail when no voltage is converted.
