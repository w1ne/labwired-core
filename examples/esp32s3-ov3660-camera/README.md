# ESP32-S3 N16R8 + OV3660 camera + 1.9" ST7789

The common ESP32-S3-N16R8 camera dev board (OV3660 on the module's DVP
ribbon, two USB-C ports) with a 1.9" 170×320 ST7789 TFT breakout wired to the
right-hand header. Both sketches are plain Arduino-ESP32 code using
Espressif's `esp32-camera` driver; nothing in them knows it is simulated.

| Sketch | What it does | Marker |
|---|---|---|
| `src/main.ino` | Live preview: grabs 160×120 RGB565 frames (two PSRAM buffers, `CAMERA_GRAB_LATEST`) and streams each to the ST7789 with `SPI.writeBytes`. | `CAMERA_PREVIEW_OK` after 3 frames |
| `take_picture/take_picture.ino` | Espressif's `take_picture` example (RGB565 QVGA, one PSRAM buffer, `CAMERA_GRAB_WHEN_EMPTY`), ported 1:1 apart from the pins. | `TAKE_PICTURE_OK` after 3 frames |
| `take_picture_jpeg/take_picture_jpeg.ino` | The same example with `PIXFORMAT_JPEG`; hex-dumps the first frame between `JPEG_BEGIN` / `JPEG_END` so it can be decoded on the host. | `TAKE_PICTURE_JPEG_OK` after 3 frames |

## Wiring

See the header of [`system.yaml`](system.yaml). The camera uses the board's
own DVP pins (GPIO 4–13, 15–18 on the left header); the display goes on the
right header (SCK 39, MOSI 40, CS 41, DC 42, RES 47, BLK 21) so it does not
share a pin with the camera bus. The camera pins are Espressif's
`CAMERA_MODEL_ESP32S3_EYE` map from arduino-esp32's `camera_pins.h` (XCLK 15,
SIOD 4, SIOC 5, Y2–Y9 = 11 9 8 10 12 18 17 16, VSYNC 6, HREF 7, PCLK 13, no
PWDN/RESET), which is the model these ESP32-S3 N16R8 camera boards tell you
to select and which matches the board's silkscreen. A board that uses a
different map only needs the `CAM_PIN_*` defines changed.

## Running it in the simulator

Build either sketch with arduino-esp32 for `esp32-s3-devkitc-1-n16r8` (PSRAM
enabled, `espressif/esp32-camera`), merge the bootloader, partition table,
`boot_app0` and app into one flash image, then:

```bash
LABWIRED_ESP32S3_FLASH=flash.bin labwired run \
  --chip configs/chips/esp32s3.yaml \
  --system examples/esp32s3-ov3660-camera/system.yaml \
  --rom-boot --firmware flash.bin --max-steps 800000000 \
  --stop-on CAMERA_PREVIEW_OK --display-out preview.ppm
```

`take_picture` prints, for each frame:

```
sensor PID=0x3660
Picture taken! Its size was: 153600 bytes (320x240 fmt=0) px0=ffff mid=f81f
```

320 × 240 × 2 bytes; pixel 0 is the white bar and the centre pixel the
magenta one, in the big-endian RGB565 the sensor puts on the wire. The
preview leaves the colour-bar frame centred on the panel.

## The picture

The twin has no lens, so the OV3660 renders a synthetic scene from its
stimulus channels:

| Channel | Range | Meaning |
|---|---|---|
| `pattern` | 0–3 | 0 colour bars, 1 checkerboard, 2 gradient, 3 flat |
| `brightness` | 0–100 % | scales every pixel |
| `motion` | 0–64 px/frame | scrolls the scene so successive frames differ |

The sensor's own colour-bar test pattern (`set_colorbar(1)`, register
0x503D bit 7) overrides the stimulus, as it does on the real part.

With `PIXFORMAT_JPEG` the sensor sends real baseline JPEG frames (YCbCr
4:2:2) of the same scene, quantised by the driver's `jpeg_quality`
(register 0x4407). `take_picture_jpeg` prints

```
Picture taken! Its size was: 4780 bytes (320x240 fmt=4) head=ffd8ff tail=ffd9
```

and the dumped frame decodes with any JPEG decoder (Pillow shows the eight
colour bars). The JPEG is an encoding of the synthetic scene, not of the
OV3660's own compression engine, so file sizes are representative rather
than bit-exact.
