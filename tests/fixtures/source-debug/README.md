# Source-debugging fixtures

`main.ino` is one Arduino sketch: a `loop()` line that calls a function which
calls another, a `Serial.println`, and a `delay`. The source-level debugger
tests in `crates/wasm/src/source_debug.rs` set breakpoints on its lines and
step over, into and out of those calls.

| File | Built by | Notes |
|---|---|---|
| `esp32-arduino.elf` | LabWired hosted compile, board `esp32dev`, entry `src/main.ino` | Classic ESP32 (Xtensa LX6). `loop()` runs on core 1. |
| `esp32s3-arduino.elf`, `esp32s3-arduino-flash.bin` | hosted compile, board `esp32-s3-devkitc-1` | ESP32-S3 (Xtensa LX7). The flash image is the build's bootloader, partition table, boot_app0 and app merged at 0x0/0x8000/0xe000/0x10000, ending at the app. |
| `esp32c3-arduino.elf`, `esp32c3-arduino-flash.bin` | hosted compile, board `esp32-c3-supermini` | ESP32-C3 (RISC-V), merged the same way. |
| `uno-arduino.elf` | local PlatformIO, `atmelavr` / `uno`, with `LED` = 13 | `build_flags = -g` plus `-g` on the LTO link (`env.Append(LINKFLAGS=["-g"])`). Hosted AVR builds link without `-g`, so they carry no line table for the sketch. |

The ESP ELFs had their debug sections compressed
(`objcopy --compress-debug-sections=zlib`), which the loader reads
transparently; that keeps each one near 2.5 MB instead of 6-7 MB.
