# ESP32-S2-DevKitC-1-N8R2

Demand rank: **#1 — ESP32 family (38% of respondents use ESP32)**. This adds
the ESP32-S2 family with a narrow UART0 profile on Espressif's current
8 MiB flash / 2 MiB PSRAM DevKitC-1 variant.

## Pin map and modeled path

| Board signal | MCU signal | LabWired connection |
| --- | --- | --- |
| USB-to-UART bridge TX/RX | UART0 RX/TX (module nets U0RXD/U0TXD) | UART0 ↔ `host_console` |

The official schematic labels U0TXD/U0RXD on the module and the J3 header and
shows the USB-to-UART bridge. The bridge is represented by `host_console`; the
CP2102N is not a separate device model.

The committed firmware writes `ESP32S2_DEVKITC_N8R2_UART_OK` to the S2's UART0
FIFO at `0x3F400000`. Run the local board smoke with:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s2-devkitc-1-n8r2/io-smoke.yaml \
  --output-dir out/boards/esp32s2-devkitc-1-n8r2/io-smoke \
  --no-uart-stdout
```

Rebuild the committed ELF fixture with `examples/esp32s2-devkitc-1-n8r2/build-firmware.sh`.

## Limits

This is a direct ELF fast-boot profile. It models the S2 LX7 execution path,
internal SRAM windows and UART0 TX FIFO. The mask ROM, flash-cache/MMU boot,
UART baud timing and RX, GPIO matrix/pad routing, RGB LED, USB OTG, Wi-Fi,
PSRAM controller, buttons and board power behavior are outside this profile.
The smoke is simulator evidence, not a physical-board test.
