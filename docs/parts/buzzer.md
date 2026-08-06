# Buzzer

Simple active/passive buzzer driven from GPIO.

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `buzzer` |
| Bus | GPIO |
| Address / select | n/a |
| Source | catalog `buzzer` |
| Tier | See matrix below |

## Pins / attachment

| Pin / net | Role |
|-----------|------|
| Signal | GPIO |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| On/off / tone indication | ✅ | Digital |
| Acoustic fidelity | ❌ | Not audio SPICE |

## How to run

1. Place part + MCU in Playground (or system YAML).
2. Agent: `labwired_list` → `labwired_describe id=buzzer` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. `labwired_describe id=buzzer`.

## Related

- [Parts index](index.md)
- Board example: [ESP32-C3](../boards/esp32c3.md)
- [Fidelity](../fidelity.md)
