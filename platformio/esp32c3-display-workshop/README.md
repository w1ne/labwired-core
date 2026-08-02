# ESP32-C3 display workshop firmware

This project builds the Arduino workshop sketch at
[`examples/esp32c3-display-workshop-arduino/`](../../examples/esp32c3-display-workshop-arduino/)
for both supported SSD1306 panel heights.

Run it from a full LabWired checkout, rather than from a copied `platformio/`
subdirectory: `platformio.ini` deliberately resolves `src_dir` to the sibling
workshop-sketch directory above.

The commands below were tested with PlatformIO Core 6.1.19. The project pins
the ESP32 platform to `espressif32@7.0.1` in `platformio.ini`.

```sh
cd platformio/esp32c3-display-workshop
pio run -e oled_128x64
pio run -e oled_128x32
```

The resulting ELF and uploadable application binary are emitted under
`.pio/build/<environment>/` as `firmware.elf` and `firmware.bin`.
