# Button

Momentary push button for GPIO input stimuli.

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `button` |
| Bus | GPIO |
| Address / select | n/a |
| Source | `button` component |
| Tier | See matrix below |

## Pins / attachment

| Pin / net | Role |
|-----------|------|
| A/B | To GPIO / GND |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| Click / level to GPIO | ✅ | Agent stimuli + UI |
| Bounce physics | ⚠️ | Idealized |

## How to run

1. Place part + MCU in Playground (or system YAML).
2. Agent: `labwired_list` → `labwired_describe id=button` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. Drive with playground click or `labwired_run` stimuli.

## Related

- [Parts index](index.md)
- Board example: [ESP32-C3](../boards/esp32c3.md)
- [Fidelity](../fidelity.md)
