# External components: ESP32-S2-DevKitC-1-N8R2

The UART smoke needs no external device model. The separate
[BME280 profile](../esp32s2-devkitc-1-n8r2-bme280/README.md) connects an
external sensor model to I2C0. The board's UART0 is physically connected to a
USB-to-UART bridge, but this simulator profile captures UART0 directly and does
not emulate the bridge or USB host connection.

The N8R2 module also includes 2 MB PSRAM and Wi-Fi/USB-capable SoC features.
They are outside the implemented profile and are not represented as working
components. See [README.md](README.md) for the supported subset.
