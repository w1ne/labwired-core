# Seven-segment display

Seven-segment digit display (GPIO or shift-register driven).

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `seven-segment` |
| Bus | GPIO / SPI shift |
| Address / select | n/a |
| Source | catalog + kits (e.g. HC595 paths) |
| Tier | See matrix below |

## Pins / attachment

| Pin / net | Role |
|-----------|------|
| Segments / DIG | Per board wiring |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| Digit visual | ✅ | Playground |
| Multiplex timing edge cases | ⚠️ | Check demos |

## How to run

1. Place part + MCU in Playground (or system YAML).
2. Agent: `labwired_list` → `labwired_describe id=seven-segment` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. `labwired_describe id=seven-segment`.

## Related

- [Parts index](index.md)
- Board example: [ESP32-C3](../boards/esp32c3.md)
- [Fidelity](../fidelity.md)
