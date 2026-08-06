# SG90 / hobby servo

Hobby servo angle via PWM-style control.

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `servo` |
| Bus | GPIO PWM / pulse |
| Address / select | n/a |
| Source | Servo kit in catalog |
| Tier | See matrix below |

## Pins / attachment

| Pin / net | Role |
|-----------|------|
| Signal | GPIO PWM |
| V+/GND | Power |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| Angle / visual | ✅ | Playground |
| Torque / analog load | ❌ | Bench |

## How to run

1. Place part + MCU in Playground (or system YAML).
2. Agent: `labwired_list` → `labwired_describe id=servo` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. `labwired_describe id=servo`.

## Related

- [Parts index](index.md)
- Board example: [ESP32-C3](../boards/esp32c3.md)
- [Fidelity](../fidelity.md)
