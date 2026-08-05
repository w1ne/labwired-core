# Part page template (external component)

**Do not publish this file as a part.** Create `parts/<id>.md` and add it under **Parts** in `mkdocs.yml`.

## Title

One line: what it is (e.g. "SSD1306 128×64 I2C OLED").

## Status at a glance

| Aspect | Status |
|--------|--------|
| Device descriptor | `configs/devices/<id>.yaml` or Rust kit path |
| Buses | I2C / SPI / GPIO — addresses, CS |
| Playground type id | catalog id |
| Example system | `configs/systems/...` |
| Tier | modeled / partial / stub |

## Pins / bus attachment

Table: component pin → typical MCU pin / net.

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| Register map / protocol | ✅/⚠️/❌ | |
| Visual in playground | ✅/⚠️/❌ | |
| Noise / SimInput | ✅/⚠️/❌ | |
| Power / analog | ❌ | usually bench |

## How to run

Minimal system yaml or playground steps + MCP `labwired_describe` id.

## Related boards

Link board pages that demos use (e.g. [ESP32-C3](../boards/esp32c3.md)).

Exemplar board shape: [ESP32-C3](../boards/esp32c3.md).
