# Arduino-ESP32 e-paper e-reader (digital twin)

A real [GxEPD2](https://github.com/ZinggJM/GxEPD2) Arduino-ESP32 e-paper sketch
for an ESP32-WROOM-32 + Waveshare 2.9" tri-color panel. The **same**
`firmware.elf` that PlatformIO builds here:

* runs unmodified in the LabWired simulator (cycle-accurate Xtensa LX6 + a
  protocol-decoding e-paper panel model), painting the page exactly as it would
  on glass — no board, no panel;
* flashes to physical ESP32-WROOM-32 hardware;
* is regression-gated headless in CI.

```
GxEPD2 sketch --pio run--> firmware.elf --> LabWired sim --SPI--> e-paper panel
                                          (Xtensa + FreeRTOS)        (rendered page)
```

Pin map (Arduino-ESP32-compatible Waveshare default):
`GPIO5 CS · GPIO17 DC · GPIO16 RST · GPIO4 BUSY · GPIO18 SCK · GPIO23 MOSI`.

## Build the firmware

```sh
pio run                 # produces .pio/build/esp32dev/firmware.elf
```

Stock setup — `platform = espressif32`, `framework = arduino`, GxEPD2 pulled
from the registry (see `platformio.ini`). The ELF is the same image you'd flash
to the board.

## Run it on the digital twin

Unlike bare-metal Cortex-M firmware (which runs via `labwired test`), a full
Arduino-ESP32 + FreeRTOS image is brought up through the simulator's
Arduino-ESP32 boot harness. The end-to-end test boots the ELF, runs FreeRTOS to
the Arduino `loopTask`, executes `setup()`, and asserts the panel paints:

```sh
LABWIRED_EREADER_ELF=examples/platformio/esp32-epaper-ereader/.pio/build/esp32dev/firmware.elf \
  cargo test -p labwired-core --test e2e_labwired_ereader -- --ignored --nocapture
# → panel refresh_gen >= 1 (the page rendered)
```

The same firmware also renders live in the browser
[Playground](https://app.labwired.com/).

## Notes

* HW-validated: the boot path was diffed against a physical ESP32-WROOM-32 over
  its UART (same banner, same `setup()` trace).
* Driver class `GxEPD2_290_C90c` (UC8151D). Picking the wrong driver makes the
  panel report success but render blank — caught in the sim, not on glass.

See the writeup:
[Render your Arduino e-paper firmware without the hardware](https://labwired.com/blog/arduino-epaper-digital-twin.html).
