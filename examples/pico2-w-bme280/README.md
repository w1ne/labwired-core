# Raspberry Pi Pico 2 W + BME280

This profile runs the RP2350 Pico 2 W with an external BME280 breakout. The
smoke firmware reads the sensor ID and raw temperature registers over I2C0,
then reports the result over UART0. It exercises the modeled RP2350 I2C0,
pad selection, UART0, and BME280 register model.

**Demand:** RP2040/RP2350 is rank #5 in the measured MCU-use survey (12%). This
adds a board system for the current Pico 2 W; it does not add a new MCU model.

## Availability checked 2026-11-10

- Raspberry Pi's [Pico 2 W Product Information Portal](https://pip.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w)
  lists board SKUs SC1633 and SC1634 and current product documents. The
  [official Pico 2 W datasheet](https://datasheets.raspberrypi.com/picow/pico-2-w-datasheet.pdf)
  states availability is guaranteed until at least January 2028 and specifies
  2 MB of on-board QSPI flash.
- Mouser's [Pico 2 series listing](https://www.mouser.com/en/c/embedded-solutions/?m=Raspberry%20Pi&series=Pico%202)
  listed SC1633 at US$7.00 with 2,525 in stock when checked.
- Adafruit's [BME280 breakout, PID 2652](https://www.adafruit.com/product/2652)
  was listed at US$14.95 and in stock. Its guide documents the default I2C
  address `0x77` used by this profile.

## Wiring

| BME280 breakout | Pico 2 W |
| --- | --- |
| VIN / 3V | 3V3(OUT), physical pin 36 |
| GND | GND, physical pin 38 |
| SDA | GP4 / I2C0 SDA, physical pin 6 |
| SCL | GP5 / I2C0 SCL, physical pin 7 |

The Pico 2 W shares the Pico 2 header layout. The [official datasheet](https://datasheets.raspberrypi.com/picow/pico-2-w-datasheet.pdf)
identifies GPIO24, GPIO25, and GPIO29 as wireless-module signals, and WL_GPIO0
as the user LED connection. This profile exposes only GP4 and GP5; it does not
claim CYW43439 Wi-Fi/Bluetooth, the wireless LED, USB boot, or power behavior.

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id pico2-w-bme280 \
  --crate firmware-pico2-w-bme280-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2-w-bme280/onboarding-smoke.yaml \
  --system configs/systems/pico2-w-bme280.yaml \
  --firmware-build-dir crates/firmware-pico2-w-bme280-smoke \
  --out-dir examples/pico2-w-bme280/evidence
```

The committed evidence records the build and local-engine smoke. This is
simulation evidence for the listed register-level slice, not a physical board
capture or a hosted `labwired_verify` verdict.
