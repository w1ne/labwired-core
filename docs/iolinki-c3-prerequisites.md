# ESP32-C3 IO-Link firmware prerequisites

The native ESP-IDF 5.4 customer L6362A reference boots through the genuine C3
mask ROM and second-stage bootloader on the `esp32c3` descriptor. Its actual
`app_main` and shared reference sampling function execute, the SDK UART1 driver
and event queue initialize, and the TX guardian has no latched fault. GPIO4
begins as a GPIO-matrix SIO output, with EN on GPIO6 enabled by the firmware.

An externally applied falling edge on OL/GPIO7 latches GPIO status. The
firmware's own ESP-IDF GPIO ISR clears it; its callback causes COM2 setup,
GPIO4 switches to U1TXD signal 9, UART1 RX selects GPIO5, and UART1 configures
8 data bits, even parity and one stop bit. The GPIO model exports the real
interrupt-matrix source 16 and gates PCPU status with each pin's enable bit.
Focused regressions cover raw masked pending, polarity and selective W1C.
The GPIO-only repair follows [Espressif's C3 TRM](https://www.espressif.com/sites/default/files/documentation/esp32-c3_technical_reference_manual_en.pdf)
and the [ESP-IDF 5.4 C3 GPIO HAL](https://github.com/espressif/esp-idf/blob/v5.4/components/hal/esp32c3/include/hal/gpio_ll.h).

This is firmware startup/interrupt evidence. Electrical C/Q, L6362A current
limiting/propagation, analog cable/supply faults, master exchanges and physical
hardware remain unverified. The added GPIO interrupt path supports configured
rising/falling/any edges on GPIO0..21; level/NMI interrupts, input
synchronizer latency and light-sleep wake-up are not modeled by this repair.
It uses the existing C3 ROM/peripheral models, whose other limits still apply.
No host IO-Link stack, native C bridge or STM32 adapter participates.

Build the [pinned customer source](https://github.com/w1ne/iolinki/tree/5572a628b8befa6a9c74496311da2359868f96d9/examples/esp32_l6362a)
with PlatformIO Core 6.1.19. Its manifest pins espressif32 6.10.0 and ESP-IDF
package 3.50400.0 (IDF 5.4.0), board `esp32-c3-devkitm-1`:

```sh
platformio run -d ../iolinki/examples/esp32_l6362a -e esp32c3
export IOLINKI_C3_ELF=/absolute/path/to/esp32c3/firmware.elf
cargo test -p labwired-core --test esp -- --include-ignored c3_gpio_interrupts:: c3_reference_firmware::
cargo test -p labwired-core --features event-scheduler --test esp -- --include-ignored c3_gpio_interrupts:: c3_reference_firmware::
```

`bootloader.bin`, `partitions.bin` and `firmware.bin` must be siblings of the
ELF. The test validates the application image's entry and every load segment
against the exact ELF bytes, including esptool's ELF SHA-256 app-description
patch, then executes that image through the genuine flash/ROM path. ROM
provisioning uses the engine's normal toolchain/vendored images. Missing inputs
fail the explicitly invoked test; ordinary host runs ignore the external build.
The prerequisite CI job rebuilds these pinned sources, requires both execution
modes and archives all images plus the map. CI execution is separate from the
local result and remains pending when this change is first proposed.

Local witness on 2026-10-01: ELF SHA-256
`846d70dbe3cd92184d117cdbfab839a577362e24a05c6a7996527e90e7f0c00b`.
This identifies that build, not future compilers' output.
