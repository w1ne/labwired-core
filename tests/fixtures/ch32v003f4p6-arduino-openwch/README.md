# CH32V003F4P6 official openwch Arduino core fixture

Built with the PlatformIO `ch32v` platform 1.1.0 (Community-PIO-CH32V),
`board = ch32v003f4p6_evt_r0`, `framework = arduino`,
`board_build.core = openwch`. The Arduino core is
`framework-arduino-openwch-ch32` at
`3820604346d0a9f0ee9a72cab8b7583e488e6674` (Community-PIO-CH32V mirror of
github.com/openwch/arduino_core_ch32). The sketch is not modified for the
simulator.

What the sketch exercises:

- the core's startup and `SystemInit` (HSI PLL to 48 MHz);
- `Serial` (HardwareSerial on USART1, TX PD5, 115200 8N1);
- `millis()` / `delay()`: the core counts `msTick` in `SysTick_Handler`
  (STK, vector 12), declared `WCH-Interrupt-fast`, so the handler saves no
  registers and relies on the QingKe hardware prologue/epilogue (HPE);
- `pinMode` / `digitalWrite` / `digitalRead` on PC0.

The sketch measures four `delay(10)` calls with `millis()` and prints
`CH32V003_ARDUINO_PASS` only when the elapsed time is 40..42 ms.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`.
Smoke: `examples/ch32v003f4p6/arduino-openwch-smoke.yaml`.
