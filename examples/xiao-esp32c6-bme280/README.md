# Seeed XIAO ESP32-C6 + BME280

This example boots Arduino firmware on the ESP32-C6FH4 profile, reads a BME280
over the XIAO's D4/D5 I²C pins, reports over native USB Serial/JTAG, and blinks
the on-board status LED on GPIO15. The board is in the ESP32 family, demand rank
**#1** (38% in the JetBrains survey); this is a new board profile for an
existing C6 model, not a new chip model.

## Run the LabWired smoke

```sh
examples/xiao-esp32c6-bme280/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/xiao-esp32c6-bme280/bme280-smoke.yaml \
  --output-dir out/xiao-esp32c6-bme280 \
  --no-uart-stdout
```

The script asserts the BME280 identity and raw temperature over I²C0 and
captures GPIO15 LED transitions. Its UART marker is carried by the XIAO's
native USB Serial/JTAG block, matching the board's Type-C connection.

## Copy to a physical board

Connect the Adafruit BME280 STEMMA QT breakout to the XIAO's D4/SDA and D5/SCL
pads (plus 3V3 and GND). The Arduino sketch in
[`firmware/src/main.cpp`](firmware/src/main.cpp) can be built and uploaded with
PlatformIO:

```sh
pio run -d examples/xiao-esp32c6-bme280/firmware -e xiao_c6 -t upload
```

Open a 115200 baud USB serial monitor. The expected line begins
`XIAO_ESP32C6_BME280_OK ID=0x60`; the on-board GPIO15 indicator blinks after
the sensor read succeeds. Arduino-ESP32 3.x and PlatformIO's
`seeed_xiao_esp32c6` board definition are used.

See [`vendor-status.md`](vendor-status.md) for the current buyability check and
[`VALIDATION.md`](VALIDATION.md) for model limits and evidence.
