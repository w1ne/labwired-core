# esp32c6-stock-arduino-gpio2.elf

Stock Arduino-ESP32 3.x sketch for the ESP32-C6-DevKitC-1, stripped
(`riscv32-esp-elf-strip`; the section table, including `.flash.appdesc`, is
kept).

```cpp
#include <Arduino.h>
void setup() { pinMode(2, OUTPUT); digitalWrite(2, HIGH); }
void loop() { delay(1000); }
```

```ini
[env:sim]
platform = espressif32
board = esp32-c6-devkitc-1
framework = arduino
```

Built with `pio run`. The image calls mask-ROM `rtc_get_reset_reason`
(`0x4000_0018`) at step 21, so it runs only when the `rom` window holds the
ROM image, and it reaches `setup()` only when the app is mapped through the
SPI0 MMU the way the 2nd-stage bootloader maps it. GPIO2 goes high near cycle
554,514.

Used by `crates/core/tests/esp/esp32c6_stock_arduino_fast_boot.rs` and the
`labwired-wasm` test `esp32c6_stock_arduino_elf_boots_with_rom_blob`.
