# Parts (external components)

External devices you wire to an MCU in the Playground or system YAML — sensors, displays, actuators, and (when documented) network helpers.

**Board pages** describe the MCU. **Part pages** describe everything else.

!!! tip "Template"
    Authors: copy [parts/_TEMPLATE.md](_TEMPLATE.md). Tone and matrix style match [ESP32-C3](../boards/esp32c3.md).

---

## How to use parts from an agent

1. `labwired_list` with `kind=component` (or search)
2. `labwired_describe` with the catalog id
3. Wire in a diagram → `labwired_validate` → `labwired_run` / `labwired_verify`

Do not invent I2C addresses or pin names — use describe / part pages.

---

## Catalog (v1 pages)

Full device models live in engine configs (`configs/devices/`, Rust kits) and the product catalog. **Public part pages** land here as they are written.

| Category | Planned public pages (parity / playground) |
|----------|--------------------------------------------|
| Sensors | MCP9808, BMP280, MPU6050, MMA8451Q, … |
| Displays | SSD1306, LCD1602, seven-segment, … |
| Actuators | SG90, LED / RGB, APA102, buzzer, … |
| Inputs | Button, switch |
| Network | RF / Wi-Fi helpers when product-ready |

Until a part page exists: use `labwired_describe`, board examples under `examples/`, and [I2C sensor tutorial](../examples/i2c_sensor_example.md).

---

## Related

- [Boards](../boards/esp32c3.md)
- [MCP tools](../agent/tools.md)
- [Simulating sensors (I2C)](../examples/i2c_sensor_example.md)
