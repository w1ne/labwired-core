# Raspberry Pi Pico 2 + BME280

This board-level profile covers the RP2350-based Raspberry Pi Pico 2 with an
external BME280 breakout at I²C address `0x77`. It reuses the engine's
behavioral RP2040 DesignWare I²C model on the RP2350 I²C0 register window and
the existing BME280 model. The firmware reads the sensor ID and the three raw
temperature bytes, reports them on UART0, and toggles the Pico 2 user LED.

## Demand and availability

This is a Tier A/B sequence item 2 extension: the measured RP2040 ecosystem is
demand rank 5 (12% usage); RP2350 is the current new flagship in that board
family. It is board/system coverage using existing I²C and BME280 models, not a
new silicon-family or sensor-model claim.

Availability checked live on 2026-11-06:

- Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
  says “Available now from $5” and that Pico 2 series will remain in production
  until at least January 2040.
- [SparkFun's Pico 2 product listing](https://www.sparkfun.com/raspberry-pi-pico-2.html)
  listed Raspberry Pi Pico 2 at $5.75 and “In stock”.
- The [Adafruit BME280 STEMMA QT breakout](https://www.adafruit.com/product/2652)
  was listed in stock. Adafruit's [pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts)
  says its default I²C address is `0x77`, matching this system profile.

## Wiring

| BME280 breakout | Pico 2 |
| --- | --- |
| VIN / 3V | 3V3(OUT), physical pin 36 |
| GND | GND, physical pin 38 |
| SDA | GP4, physical pin 6 |
| SCL | GP5, physical pin 7 |

Raspberry Pi's [Pico 2 pinout](https://datasheets.raspberrypi.com/pico/Pico-2-Pinout.pdf)
lists GP4 as I²C0 SDA and GP5 as I²C0 SCL. The simulation checks the modeled
register-level transaction; power, pull-up behavior, and physical signal
integrity are outside this profile.

## Run the smoke

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id pico2-bme280 \
  --crate firmware-pico2-bme280-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2-bme280/onboarding-smoke.yaml \
  --system configs/systems/pico2-bme280.yaml \
  --firmware-build-dir crates/firmware-pico2-bme280-smoke \
  --out-dir examples/pico2-bme280/evidence
```

The smoke expects `BME280_ID=0x60`, the modeled default raw temperature sample
`0x082DC0`, the completion marker, and at least eight GP25 LED edges. The
committed `evidence/` directory records the build and local-engine run.

## Scope

- Modeled: I²C0, IO_BANK0 function selection on GP4/GP5, BME280 register reads,
  UART0 data-register output, SIO GP25 output, and the Pico 2 LED connection.
- Not claimed: RP2350 Hazard3 execution, TrustZone, PIO v2, USB boot/CDC,
  physical UART waveform, electrical/power behavior, or wireless functions.
- This is repository simulation evidence; no physical Pico 2 capture exists.
