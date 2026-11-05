# NUCLEO-F411RE + MCP9808

This design attaches Adafruit's MCP9808 temperature-sensor breakout to the
NUCLEO-F411RE's I2C1 bus. The firmware reads the sensor's manufacturer ID,
device ID, and ambient-temperature register, then prints a pass marker only
when the IDs match the MCP9808 model and the temperature is near its default
25 °C input.

## Wiring

| MCP9808 breakout | NUCLEO-F411RE |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | D14 / PB9 |
| SCL | D15 / PB8 |

`Wire.begin()` uses I2C1 on PB9/PB8 in the NUCLEO-F411RE Arduino variant. The
system descriptor attaches the MCP9808 model at its default address, `0x18`.
USART2 on PA2/PA3 captures the result through the modeled ST-LINK VCP path.

## Reproduce

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f411re-mcp9808 \
  --crate examples/nucleo-f411re-mcp9808 \
  --firmware-build-script scripts/build_nucleo_f411re_mcp9808_smoke.sh \
  --script examples/nucleo-f411re-mcp9808/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f411re-mcp9808.yaml \
  --out-dir examples/nucleo-f411re-mcp9808/evidence
```

Build and simulation logs are committed in [`evidence/`](evidence/).
The live availability check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The focused F411RE chip profile models RCC, flash control, PWR, GPIOA/GPIOB,
SysTick, USART2, and I2C1. The attached MCP9808 device uses the existing
register model in `configs/devices/mcp9808.yaml`. Its alert output and
conversion-time gating are not modelled, and its configured temperature input
currently covers non-negative values only. Unused F411RE peripherals are
omitted from this profile and unsupported; the smoke is simulator evidence,
not hardware confirmation.
