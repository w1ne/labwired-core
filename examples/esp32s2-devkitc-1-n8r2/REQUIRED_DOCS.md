# Required documentation: ESP32-S2-DevKitC-1-N8R2

The supported surface and simulator limitations are in [README.md](README.md)
and [VALIDATION.md](VALIDATION.md). The live buyability check is recorded in
[vendor-status.md](vendor-status.md). This UART profile has a committed UART0
smoke and fixture. The separate BME280 system is documented in
[the I2C example](../esp32s2-devkitc-1-n8r2-bme280/README.md). Neither profile
claims ROM boot or full SoC coverage.

## Firmware

`firmware/main.c` is a copy-pasteable bare-metal UART0 example for a firmware
image linked at the S2 IROM address. It configures the UART divisor and writes
`S2_UART_OK` to the UART0 FIFO. `firmware/build-firmware.sh` builds the
committed simulator fixture using the Espressif Xtensa toolchain.

## Evidence

See [VALIDATION.md](VALIDATION.md) and the committed `evidence/` directory for
the build and local simulator smoke logs. The UART block is shared with the
existing Espressif UART model; the S2 address map and fast-start path are
implemented in this repository.
