# Test fixture: two ESP32-C3s on one UART wire

Two C3 nodes cross-linked on UART1, sending `PING` and `PONG`. This is what CI
runs to check that cross-chip serial works on a chip with a GPIO matrix.

It is a fixture, not a lab. The version people are meant to read and build is
`examples/esp32c3-pingpong`, which is Arduino and draws the rally on an OLED.

It is separate because CI has to build it with nothing installed. The firmware is
bare-metal Rust for `riscv32imc-unknown-none-elf`, a target that comes with
rustup. No Espressif toolchain, no ESP-IDF, no PlatformIO, no network. The
Arduino sketches cannot meet that, since compiling them needs the hosted builder.

Bare-metal is also deliberate. The firmware writes the C3's UART registers
directly, so if the `esp_uart` model breaks, this test catches it instead of a
HAL hiding the problem.

```
cargo build --release          # writes firmware/{server,client}.elf
```

The assertions are in `crates/core/tests/world_esp32c3_pingpong.rs`.
