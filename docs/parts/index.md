# Parts (external components)

External devices you wire to an MCU in the Playground or system YAML.

!!! tip "Template"
    Authors: [parts/_TEMPLATE.md](_TEMPLATE.md). Board tone: [ESP32-C3](../boards/esp32c3.md).

## Agent path

1. `labwired_list` (`kind=component` if supported)
2. `labwired_describe` with catalog id
3. `labwired_validate` → `labwired_run` / `labwired_verify`

## Catalog (documented)

| Part | Catalog id | Bus |
|------|------------|-----|
| [MCP9808](mcp9808.md) | `mcp9808` | I²C |
| [MMA8451Q](mma8451q.md) | `mma8451q` | I²C |
| [MPU-6050](mpu6050.md) | `mpu6050` | I²C |
| [BMP280](bmp280.md) | `bmp280` | I²C (primary in playground) |
| [SSD1306 OLED](ssd1306.md) | `oled-ssd1306 / oled-ssd1306-128x32` | I²C |
| [LCD1602](lcd1602.md) | `lcd1602` | GPIO parallel / I²C backpack variants |
| [APA102 / DotStar](apa102.md) | `apa102` | SPI-like (data + clock) |
| [SG90 / hobby servo](servo.md) | `servo` | GPIO PWM / pulse |
| [Button](button.md) | `button` | GPIO |
| [Buzzer](buzzer.md) | `buzzer` | GPIO |
| [Seven-segment display](seven-segment.md) | `seven-segment` | GPIO / SPI shift |

More devices exist in engine configs and the product catalog — if a part is missing here, describe still works when the id is in the live catalog.

## Related

- [Boards](../boards/esp32c3.md) · [MCP tools](../agent/tools.md) · [I²C tutorial](../examples/i2c_sensor_example.md)
