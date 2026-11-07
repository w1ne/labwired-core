# ESP32-C6-DevKitM-1-N4

Demand rank: **ESP32 family #1** (38% in the JetBrains 2025 MCU-use survey).
This adds exact board coverage for Espressif's current 4 MB DevKitM variant;
it reuses the existing ESP32-C6 core and peripheral models.

The focused system descriptor models 4 MB flash, the HP core, UART0 console
capture, GPIO8, RMT TX channel 0, PCR clock gates, and the onboard WS2812B
addressable LED. The LED pin and device type come from the live Espressif guide
and official schematic. The boot smoke asserts UART output and the red RGB
pixel. See [the example and committed evidence](../../examples/esp32c6-devkitm1-n4/README.md).

The descriptor does not claim ROM boot, LP-core execution, Wi-Fi, BLE, 802.15.4,
USB, or the rest of the board's GPIO/peripheral set. The UART is captured at
the peripheral; bridge propagation is not modeled. This is local simulator
evidence, not a silicon comparison or hardware confirmation.
