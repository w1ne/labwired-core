# ESP32-DevKitC-32E

Espressif's ESP32-DevKitC-32E is the ESP32-WROOM-32E, 4 MB flash version of
the DevKitC V4. Its narrow LabWired onboarding profile boots an Arduino GPIO
smoke and observes GPIO23, mapped by Espressif to J3 pin 2.

| Item | Coverage |
| --- | --- |
| System | [`configs/systems/esp32-devkitc-32e-gpio-smoke.yaml`](../../configs/systems/esp32-devkitc-32e-gpio-smoke.yaml) |
| Chip profile | [`configs/chips/esp32-devkitc-32e-gpio-smoke.yaml`](../../configs/chips/esp32-devkitc-32e-gpio-smoke.yaml) |
| Firmware and evidence | [`examples/esp32-devkitc-32e/`](../../examples/esp32-devkitc-32e/README.md) |
| Smoke assertion | At least 16 GPIO23 output edges |
| Demand rank | ESP32: 1 (38% in the referenced MCU survey) |

The profile models GPIO only. The USB-to-UART bridge, Wi-Fi/Bluetooth, I2C,
SPI, timers, and power LED are outside the smoke. The board has no GPIO user
LED; an external LED and resistor on GPIO23 are needed for a physical blink.
The local repository CLI smoke passes. The hosted verifier's GPIO oracle sees
the output toggle, but that run ends `unsupported` on a separate unmapped
classic-ESP32 boot read; see the recorded result in the example evidence.

Espressif's [live DevKitC V4 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
lists this exact variant and its J3 pin map. The live availability check is in
the example evidence.
