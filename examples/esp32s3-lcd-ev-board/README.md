# ESP32-S3-LCD-EV-Board v1.5 touch profile

**Demand position:** ESP32 is rank 1 in the measured MCU list (38%); ESP32
development boards account for 42% of main-project kits. This is a new
board-and-touch-controller path inside that demand group, not a new MCU.

This profile targets the ESP32-S3-LCD-EV-Board v1.5 mainboard with its SUB2
v1.5 3.95-inch, 480×480 touchscreen. It models UART0 and one FT5x06 touch
point over I²C0. The committed vector reports a finger at **(301, 77)**.

## Copy-paste Arduino example

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t TOUCH_ADDR = 0x38;
constexpr int SDA_PIN = 47;
constexpr int SCL_PIN = 48;

void setup() {
  Serial.begin(115200);
  Wire.begin(SDA_PIN, SCL_PIN, 100000);
  Wire.beginTransmission(TOUCH_ADDR);
  Wire.write(0x02); // touch count, followed by one six-byte point record
  if (Wire.endTransmission(false) != 0 || Wire.requestFrom(TOUCH_ADDR, 5) != 5) {
    Serial.println("FT5X06_I2C_ERROR");
    return;
  }

  const uint8_t count = Wire.read() & 0x0F;
  const uint8_t xh = Wire.read();
  const uint8_t xl = Wire.read();
  const uint8_t yh = Wire.read();
  const uint8_t yl = Wire.read();
  const uint8_t point_id = yh >> 4;
  const uint16_t x = (uint16_t(xh & 0x0F) << 8) | xl;
  const uint16_t y = (uint16_t(yh & 0x0F) << 8) | yl;

  if (count == 1 && point_id != 0x0F) {
    Serial.printf("TOUCH_COUNT=%u ID=%u X=%u Y=%u\n", count, point_id, x, y);
  }
}

void loop() {}
```

The official guide maps I²C SDA/SCL to GPIO47/GPIO48 on v1.5. The FT5x06
address is `0x38`; this example reads the controller's touch status and first
point fields.

## Run the committed smoke

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32s3-lcd-ev-board-v1-5-touch \
  --script examples/esp32s3-lcd-ev-board/touch-smoke.yaml \
  --system configs/systems/esp32s3-lcd-ev-board-v1-5-touch.yaml \
  --firmware-build-script examples/esp32s3-lcd-ev-board/ci/build-touch-firmware.sh \
  --out-dir examples/esp32s3-lcd-ev-board/evidence/touch
```

The evidence checker and committed output are under
[`evidence/touch/`](evidence/touch/). The simulator enters the compiled ELF's
`labwired_entry` fast-boot function and reads the same FT5x06 registers through
the ESP32-S3 I²C peripheral model. It bypasses Arduino startup; the copy-paste
sketch shows the normal `Wire` path for hardware. This is simulator evidence,
not a physical touchscreen test.

## Model limits

The FT5x06 model supports one contact point and the finger-down event code.
The model accepts 12-bit coordinates; the selected 480×480 panel uses the
screen-space range 0–479. The packed byte at register `0x05` is exposed as the
`touch1_id_yh` input: bits 7:4 are the point ID and bits 3:0 are `y[11:8]`.
Use ID `0xF` in the high nibble when no point is present.

This narrow system profile does not model the GC9503CV display controller or
RGB pixel output, the TCA9554 I/O expander, ES7210 microphone ADC, ES8311 audio
codec, NS4150 amplifier, status LED, PSRAM, USB, or board power behavior. No
display painting, audio, USB, or physical input accuracy is claimed.

See [`evidence/vendor-status.md`](evidence/vendor-status.md) for the live
board-availability receipt and exact guide sources.
