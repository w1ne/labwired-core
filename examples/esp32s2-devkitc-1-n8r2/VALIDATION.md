# Validation: ESP32-S2-DevKitC-1-N8R2 UART0

The committed bare-metal ELF starts directly at its linked entry point in the
S2 IROM address window, configures UART0, and emits `S2_UART_OK`. The simulator
smoke asserts that marker. It exercises the Xtensa LX7 instruction path,
firmware load into the S2 address windows, and the shared Espressif UART0
register model at `0x60000000`.

The board's guide assigns UART0 TX/RX to GPIO43/GPIO44 and connects those pins
to the USB-to-UART bridge. The model captures UART0 at the peripheral, so this
does not exercise the IO_MUX, pad-level UART routing, or CP2102 bridge.

The model is a fast-start subset: it skips mask-ROM execution and the SPI flash
controller/MMU setup, treats IROM/DROM as directly mapped RAM backing, and does
not provide physical flash boot. The 2 MB PSRAM, Wi-Fi, USB OTG, GPIO40..46,
interrupt matrix/UART interrupt routing, IO_MUX electrical behavior, and
GPIO18 addressable RGB LED are not modeled. GPIO0..39 and I2C0 are present for
the separate BME280 profile, but this UART-only smoke does not exercise them.
The smoke polls UART0; it does not exercise UART interrupts. Separate IROM/DRAM
backing views do not currently alias internal SRAM as they do in silicon.

The smoke is local simulator evidence, not a hardware run or silicon
correlation. Logs and machine-readable output are committed under
[`evidence/`](evidence/). The hosted LabWired board catalog had no ESP32-S2
entry when this round was run, so this board has no `labwired_verify` result.

Build and run from the repository root:

```sh
examples/esp32s2-devkitc-1-n8r2/firmware/build-firmware.sh
scripts/onboarding_smoke.sh \
  --target-id esp32s2-devkitc-1-n8r2 \
  --firmware-build-script examples/esp32s2-devkitc-1-n8r2/firmware/build-firmware.sh \
  --script examples/esp32s2-devkitc-1-n8r2/uart-smoke.yaml \
  --system configs/systems/esp32s2-devkitc-1-n8r2.yaml \
  --out-dir examples/esp32s2-devkitc-1-n8r2/evidence \
  --threshold-seconds 3600
```

Vendor availability is recorded in [vendor-status.md](vendor-status.md).
