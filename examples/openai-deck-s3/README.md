# openai-deck-s3 — ESP32-S3 OpenAI macro deck

ESP32-S3 firmware for a 10-key macro deck: a 1.5" **128×128 SH1107 OLED** over
I²C0 plus **10 momentary key switches** on GPIO inputs. Each key press emits a
host-protocol line over USB-Serial-JTAG (`KEYn PRESS action=SLOTn`) and
highlights that key's slot on the OLED. The host maps each `SLOTn` to a real
OpenAI action (filled in later — this milestone ships OLED + keys + serial only;
no potentiometer/ADC).

## Pin map

| Signal        | GPIO   | Notes                              |
|---------------|--------|------------------------------------|
| I²C0 SDA      | GPIO8  | mirrors esp32s3-i2c-tmp102         |
| I²C0 SCL      | GPIO9  |                                    |
| KEY1          | GPIO4  | active-high (idle low)             |
| KEY2          | GPIO5  |                                    |
| KEY3          | GPIO6  |                                    |
| KEY4          | GPIO7  |                                    |
| KEY5          | GPIO10 |                                    |
| KEY6          | GPIO11 |                                    |
| KEY7          | GPIO12 |                                    |
| KEY8          | GPIO13 |                                    |
| KEY9          | GPIO14 |                                    |
| KEY10         | GPIO15 |                                    |

Strapping pins (0/3/45/46), USB pins (19/20) and the SPI-flash pins (26–32) are
deliberately avoided.

## OLED address (0x3D, not 0x3C)

The simulator's ESP32-S3 system builder (`configure_xtensa_esp32s3`)
unconditionally attaches an **SSD1306 at 0x3C**, and the `Esp32s3I2c` controller
dispatches each transaction to the *first* slave that matches the address. To
attach the **SH1107 without modifying the simulator core**, this lab uses the
kit's SA0=high address **0x3D** (the documented 0x3C/0x3D pair). The firmware
addresses 0x3D accordingly. On real hardware, tie the panel's SA0 pin high.

## FP-free

The simulator does not model the Xtensa FPU, so all rendering is integer/bitmap
work (a 5×7 column-major font + page-addressed GDDRAM writes).

## Build

```
cd examples/openai-deck-s3
cargo +esp build --release
# → target/xtensa-esp32s3-none-elf/release/openai-deck-s3
```

## Run in the simulator

The native test `crates/core/tests/openai_deck_s3_boot.rs` builds this firmware,
boots the ELF on the S3 fast-boot path with the SH1107 attached to I²C0, asserts
the OLED framebuffer is non-blank after init, then drives a key GPIO high via
`set_gpio_input` and asserts the `KEY…` press line appears on serial:

```
cargo test -p labwired-core --features esp32s3-fixtures --release \
    --test openai_deck_s3_boot -- --nocapture
```

## Run on real hardware

Flash with `cargo +esp run --release` (espflash). Wire an SH1107 breakout to
GPIO8/GPIO9 (SA0 high → 0x3D) and 10 switches from the KEYn GPIOs to 3V3 with
pull-downs to ground.

## Sources

- ESP32-S3 TRM v1.4 §29 (I²C controller), §6 (GPIO / IO-MUX)
- Sino Wealth SH1107 datasheet (128×128 OLED, page addressing)
- esp-hal 1.1.0 `i2c::master::I2c`, `gpio::Input`
