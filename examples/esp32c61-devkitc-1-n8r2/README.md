# ESP32-C61-DevKitC-1-N8R2 UART0 smoke

This example boots a bare-metal RV32IMAC image on the ESP32-C61-DevKitC-1-N8R2
and writes a recognizable line through UART0. ESP32-family boards rank first in
the measured demand list (38% reported usage).

## Build and run

```sh
examples/esp32c61-devkitc-1-n8r2/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c61-devkitc-1-n8r2/uart-smoke.yaml \
  --output-dir out/esp32c61-devkitc-1-n8r2 \
  --no-uart-stdout
```

The firmware enables UART0's PCR clock, sets its divider, and writes
`ESP32C61_DEVKITC1_UART0_OK`. The model captures the UART FIFO output. This
profile intentionally exposes only UART0 and its PCR clock gate; it does not
claim pin-level routing through the USB-UART bridge, GPIO, radio, native USB,
RGB LED, or other peripherals.

See [`vendor-status.md`](vendor-status.md) for the live availability check and
[`VALIDATION.md`](VALIDATION.md) for the smoke evidence and limitations.
