# Parts (external components)

Devices you attach to an MCU in Playground, system YAML, or an agent diagram: sensors, displays, motors, buttons, and more.

**Add a new part:** [Onboard a part (I²C · SPI · actuators)](../howto/onboard-part.md)

!!! tip "Template"
    Authors: [parts/_TEMPLATE.md](_TEMPLATE.md). Tone: [ESP32-C3 board page](../boards/esp32c3.md).

---

## Agent path

1. `labwired_list` (optionally `kind=component`)  
2. `labwired_describe` with catalog id  
3. `labwired_validate` → `labwired_run` / `labwired_verify`  

---

## Catalog (documented here)

| Part | Catalog / type id | Bus |
|------|-------------------|-----|
| [MCP9808](mcp9808.md) | `mcp9808` | I²C |
| [MMA8451Q](mma8451q.md) | `mma8451q` | I²C |
| [MPU-6050](mpu6050.md) | `mpu6050` | I²C |
| [BMP280](bmp280.md) | `bmp280` | I²C |
| [SSD1306 OLED](ssd1306.md) | `oled-ssd1306` / `oled-ssd1306-128x32` | I²C |
| [LCD1602](lcd1602.md) | `lcd1602` | GPIO / I²C backpack |
| [APA102](apa102.md) | `apa102` | SPI-like |
| [Servo](servo.md) | `servo` | GPIO PWM |
| [Button](button.md) | `button` | GPIO |
| [Buzzer](buzzer.md) | `buzzer` | GPIO |
| [Seven-segment](seven-segment.md) | `seven-segment` | GPIO / SPI |

More devices live under [`configs/devices/`](../../configs/devices/) and in the live catalog. If a part is missing from this table, `labwired_describe` still works when the id is registered.

**Also useful:** TMP102 (`tmp102`), ADXL345 SPI (`adxl345_spi`), DC motor (`dc-motor`) — see [Onboard a part](../howto/onboard-part.md).

---

## Related

- [Onboard hardware](../howto/onboard-hardware.md)
- [I²C sensor example](../examples/i2c_sensor_example.md)
- [MCP tools](../agent/tools.md)
