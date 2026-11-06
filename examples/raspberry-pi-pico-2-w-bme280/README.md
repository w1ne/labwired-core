# Raspberry Pi Pico 2 W + BME280

This board-level profile attaches an external BME280 breakout to the Raspberry
Pi Pico 2 W's I²C0 route on GP4/GP5. Firmware reads the sensor ID and raw
temperature bytes and reports them through the simulator's UART0 console.

## Demand and availability

This is a Tier A/B sequence item 2 target. RP2040 measured 12% usage (survey
rank 5); RP2350 is the current flagship in that same family, with no separate
survey percentage. This adds sensor coverage to an existing Pico 2 W GPIO
profile; it does not add a new silicon family or wireless model.

Raspberry Pi's product page was checked live on 2026-11-06 and said “Available
now from $5”; it also says the Pico 2 series will remain in production until at
least January 2040. The current Pico 2 W datasheet separately guarantees
availability of the exact W board until at least January 2028. Adafruit's
BME280 STEMMA QT breakout listing was also live and marked **In stock** at
$14.95. The [vendor-status receipt](evidence/vendor-status.md) records both
products and the distinction between the series commitment and exact-board
guarantee.

## Wiring

| BME280 breakout | Pico 2 W |
| --- | --- |
| VIN / 3V | 3V3(OUT), physical pin 36 |
| GND | GND, physical pin 38 |
| SDA | GP4, physical pin 6 |
| SCL | GP5, physical pin 7 |

The sensor is configured at I²C address `0x77`. Adafruit's [pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts)
lists `0x77` as the default and `0x76` when its address jumper is changed.

## Run the smoke

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id raspberry-pi-pico-2-w-bme280 \
  --crate firmware-pico2-w-bme280-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/raspberry-pi-pico-2-w-bme280/onboarding-smoke.yaml \
  --system configs/systems/raspberry-pi-pico-2-w-bme280.yaml \
  --firmware-build-dir crates/firmware-pico2-w-bme280-smoke \
  --out-dir examples/raspberry-pi-pico-2-w-bme280/evidence
```

The smoke asserts `BME280_ID=0x60`, the modeled sample `BME280_TEMP_RAW=0x082DC0`,
and the completion marker. Build and simulator logs are committed in
[`evidence/`](evidence/).

## Scope

The profile claims the RP2350 Cortex-M33 execution path, I²C0 on GP4/GP5, the
BME280 model, and UART0 output inside the simulator. CYW43439 Wi-Fi/Bluetooth,
USB boot/CDC, the wireless-controlled onboard LED, power behavior, physical
UART wiring, and physical-board parity are not modeled or claimed. The BME280
model answers the sensor reads; the Pico 2 W radio and other omitted hardware
are outside this profile.
