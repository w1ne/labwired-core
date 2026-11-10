# Validation record

## Demand and scope

ESP32 is demand rank **#1** in the 2025 embedded MCU survey (38%). This round
adds a board-and-sensor smoke for the currently listed ESP32-C6-DevKitC-1-N8.
The C6 chip and I²C controller were already modeled; this is board/system
coverage, not a new MCU core. The system exercises UART0 and reads a BME280
over I²C0 on GPIO6/7.

## Reproduce

```sh
examples/esp32c6-devkitc-1-n8-bme280/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-devkitc-1-n8-bme280/bme280-smoke.yaml \
  --output-dir out/esp32c6-devkitc-1-n8-bme280 \
  --no-uart-stdout
```

The committed smoke transcript, structured result, and onboarding metrics are
under [`evidence/`](evidence/). The GPIO6/7 I²C assignment comes from the
LabWired board model; it is not silicon-confirmed. The Arduino board variant's
separate default `Wire` route (GPIO23/22) is not the route exercised here.

## Claim boundary

The intended pass reads BME280 `CHIP_ID=0x60`, receives raw temperature
`0x082DC0` at the model's default 25 °C, prints its UART0 marker, and then stops
at the configured instruction limit in the idle loop. This is local
digital-twin evidence using the Arduino-ELF fast-boot path; it is not a
hardware measurement or a full mask-ROM flash-image boot. Wi-Fi 6, Bluetooth,
802.15.4, native USB, and the board's WS2812 RGB LED are not exercised; the LED
is deliberately omitted from the system descriptor as a plain GPIO device.
