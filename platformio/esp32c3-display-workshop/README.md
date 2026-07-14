# ESP32-C3 display workshop firmware

This project builds the Arduino workshop sketch at
[`examples/esp32c3-display-workshop-arduino/`](../../examples/esp32c3-display-workshop-arduino/)
for both supported SSD1306 panel heights.

```sh
cd platformio/esp32c3-display-workshop
pio run -e oled_128x64
pio run -e oled_128x32
```

The resulting ELF and uploadable application binary are emitted under
`.pio/build/<environment>/` as `firmware.elf` and `firmware.bin`.
