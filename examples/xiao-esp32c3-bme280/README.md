# Seeed Studio XIAO ESP32C3 + BME280

## Demand position

ESP32 is demand rank **1** (38% of surveyed MCU use; ESP32 boards were 42% of
main-project kits). The XIAO ESP32C3 is a USD 4.90 thumb-sized ESP32-C3 board.
This adds a tested board-and-sensor profile over the existing ESP32-C3 model.
It is board depth, not new silicon. Live availability is recorded in
[vendor-status.md](vendor-status.md).

## Wiring

Use a Seeed Studio XIAO ESP32C3 and an Adafruit BME280 STEMMA QT breakout:

| Breakout | XIAO pin |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | D4 (GPIO6) |
| SCL | D5 (GPIO7) |

These are the board's default `Wire` pins. The breakout's address is 0x77.
`Serial` goes to the USB-C socket (the C3's own USB-Serial/JTAG), as on the
stock board.

## Copy-paste Arduino sketch

[`firmware/src/main.cpp`](firmware/src/main.cpp) is the full sketch. Build it
with the stock PlatformIO board, without extra flags:

```ini
[env:xiao_esp32c3_bme280]
platform = espressif32@8.0.0
board = seeed_xiao_esp32c3
framework = arduino
```

It reads CHIP_ID (0xD0), the temperature calibration (0x88..0x8D), starts one
forced measurement (0xF4 = 0x25), reads the raw temperature (0xFA..0xFC) and
prints the compensated value from the Bosch datasheet formula (section 4.2.3).

## Local smoke

The committed fixture is the unmodified Arduino build: the simulator runs the
full Arduino-ESP32 startup (no fast-boot entry), then `setup()`. The smoke
asserts on the USB console:

```
XIAO_C3_BOOT
BME280_ID=0x60
BME280_DIG_T1=28528
BME280_TEMP_RAW=0x82DC0
BME280_TEMP_CENTI_C=2500
```

2500 is 25.00 degC. A separate Python calculation of the same Bosch formula
from the raw value and the model's calibration also gives 2500.

Negative control: with the BME280 removed from the system, the same firmware
prints `BME280_I2C_ERROR` and the smoke fails 1/5.

```sh
examples/xiao-esp32c3-bme280/ci/test.sh
scripts/onboarding_smoke.sh --target-id seeed-xiao-esp32c3-bme280 --script examples/xiao-esp32c3-bme280/bme280-smoke.yaml --system configs/systems/seeed-xiao-esp32c3-bme280.yaml --firmware-build-script examples/xiao-esp32c3-bme280/ci/build-bme280-firmware.sh --out-dir examples/xiao-esp32c3-bme280/evidence
```

The fixture needs its partition table beside it
(`tests/fixtures/tier1/seeed-xiao-esp32c3-bme280.partitions.bin`); without
it, ESP-IDF stops in `esp_partition` with "No MD5 found in partition table".

## Engine fix this board needed

Before this change, `labwired test` wrote the app-image magic (0xE9) at
DROM VA 0x3C03_0000 only. ESP-IDF 5 reads it at the page of
`_rodata_reserved_start`. That page moves with the text size. This sketch
has more than 192 KiB of text, so the page is 0x3C04_0000. The boot then
stopped in `system_early_init` with "Invalid app image header" at about step
196k. The CLI now reads the page from the ELF's `.flash.appdesc` section, as
the C6 path already did.

## Limits

The profile covers the USB-Serial/JTAG console and I2C0 with this BME280
breakout. It does not claim Wi-Fi/Bluetooth, the BOOT/RESET buttons, battery
charging, board power behavior or physical sensor accuracy. The sensor model
returns a fixed default reading (25.00 degC); it is not live sensing.
