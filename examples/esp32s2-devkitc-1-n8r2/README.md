# ESP32-S2-DevKitC-1-N8R2 UART smoke

This is a demand-rank 1 **ESP32 family** extension (38% MCU usage in the
JetBrains State of Embedded survey); the survey does not rank S2 separately.
It targets the exact current ESP32-S2-DevKitC-1-N8R2 revision, not the EOL 4 MB
board variants.

The committed bare-metal ELF executes from the ESP32-S2 IROM address window,
sets UART0 to approximately 115200 baud, and sends `S2_UART_OK` through the
UART FIFO. The board uses UART0 on GPIO43/GPIO44 and connects it to the
USB-to-UART bridge. LabWired captures the modeled UART0 directly: the UART pad
mux and CP2102 bridge are not part of this model slice.

## Supported surface

- Xtensa LX7 single-core firmware entry and ESP32-S2 internal SRAM.
- ESP32-S2 IROM and DROM address windows for the bare-ELF fast-start profile.
- UART0 FIFO and serial output.
- Board UART0 pin assignment: TX GPIO43, RX GPIO44.
- Shared ESP32 GPIO0..39 model subset and the S2 I2C0 command-list model; the
  separate [BME280 smoke](../esp32s2-devkitc-1-n8r2-bme280/README.md) uses GPIO8/9.

The smoke in this directory checks UART0 only. The S2 mask ROM, SPI-flash
controller/MMU, 2 MB PSRAM, Wi-Fi, USB OTG, GPIO40..46, interrupt matrix,
UART interrupt routing, GPIO18 addressable RGB LED, IO_MUX electrical behavior,
and physical USB-to-UART bridge are not modeled. This is a simulator fast-start
ELF, not a physical flash boot or a hardware validation.

## Build and run

```bash
examples/esp32s2-devkitc-1-n8r2/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s2-devkitc-1-n8r2/uart-smoke.yaml \
  --no-uart-stdout
```

See [`VALIDATION.md`](VALIDATION.md), [`REQUIRED_DOCS.md`](REQUIRED_DOCS.md),
and [`vendor-status.md`](vendor-status.md) for evidence and source notes.
