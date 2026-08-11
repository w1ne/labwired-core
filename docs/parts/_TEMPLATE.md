# Part page template (external component)

**Do not publish this file as a part.** Create `parts/<id>.md` and add it under **Parts** in `mkdocs.yml`.

Follow [Onboard a part](../howto/onboard-part.md) for the engineering steps.

---

## Title

One line: what it is (e.g. "SSD1306 128×64 I²C OLED").

## Status at a glance

| Aspect | Status |
|--------|--------|
| Device descriptor | `configs/devices/<id>.yaml` or kit path |
| Buses | I²C / SPI / GPIO — addresses, CS |
| Playground / catalog type id | |
| Example system or lab | `configs/systems/...` or `examples/...` |
| Tier | modeled / smoke / partial / stub |

## Pins / bus attachment

| Part pin | Typical MCU net | Notes |
|----------|-----------------|-------|
| SDA | I2C SDA | |
| SCL | I2C SCL | |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| Register map / protocol | ✅/⚠️/❌ | |
| Visual in Playground | ✅/⚠️/❌ | |
| SimInput / noise | ✅/⚠️/❌ | |
| Power / analog | ❌ | usually bench |

## How to run

1. Minimal system YAML or Playground steps  
2. MCP: `labwired_describe` id=`...`  
3. Smoke command: `labwired test --script ...` or verify assertions  

## Limitations

List what is **not** modeled (from the YAML header).

## Related boards

Link board pages used by demos (e.g. [ESP32-C3](../boards/esp32c3.md)).
