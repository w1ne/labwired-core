# Arduino × LabWired board matrix — `arduino_nano_esp32`

Run 2026-11-16 with `validation/arduino-matrix/run_matrix.py --boards arduino_nano_esp32`
(L2 rerun after the `LED_BUILTIN` sketch fix). Per-cell files are in this directory.

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can |
|------|------|------|------|------|------|------|------|------|------|
| `arduino_nano_esp32` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

- Cells: **9** · **pass: 9** · **skip: 0** · **fail: 0**
- L2 oracle: 3 edges on `gpio:48` (LED_BUILTIN = D13), minimum 2.
- L6 oracle: 59 edges on `gpio:5` (D2, LEDC PWM), minimum 20.
