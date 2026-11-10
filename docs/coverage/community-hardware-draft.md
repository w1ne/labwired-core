# Community hardware quickstarts — draft

This is a review draft for the community coverage page. It is intentionally not
listed in `mkdocs.yml` and has not been published. The current draft covers the
ESP32 family and RP2350; the remaining demand-ranked entries need to be added
and reviewed before publication.

## ESP32 — rank 1, 38% of reported MCU use

### ESP32-C5-DevKitC-1 v1.2 / ESP32-C5-WROOM-1-N8R8

This is more coverage within the already top-ranked ESP32 family, not a new
board or MCU demand pick. The v1.2 board guide lists the N8R8 variant with
8 MB flash and 8 MB PSRAM, and DigiKey currently lists the exact board as
Active and in stock. The focused LabWired profile exercises UART0, a BME280 on
I2C0 (GPIO6/GPIO10), an SSD1306 OLED sharing that bus, and the onboard GPIO27
addressable RGB LED through RMT TX0. The committed local smoke checks the
BME280 ID and temperature register, a painted SSD1306 framebuffer, RMT TX
completion, GPIO edges, and a decoded red LED pixel. It does not claim a
ROM/ESP-IDF boot, physical-board result, or hosted `model_verified` result.

Copy-paste Arduino example for the onboard GPIO27 RGB LED with the Espressif
Arduino core for ESP32-C5:

```cpp
#include <Arduino.h>

void setup() {
  rgbLedWrite(27, 64, 0, 0);  // red
}

void loop() {}
```

The [ESP32-C5-DevKitC-1 v1.2 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
identifies the N8R8 module and GPIO27 LED. [DigiKey's exact N8R8 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C5-DEVKITC-1-N8R8/28718162)
showed Active status and 312 in stock when checked on 2026-11-10. The local
smoke, wiring notes and run instructions are in
[`examples/esp32c5-devkitc/`](../../examples/esp32c5-devkitc/); its detailed
availability record is in
[`evidence/vendor-status.md`](../../examples/esp32c5-devkitc/evidence/vendor-status.md).

Copy-paste Arduino example for an Adafruit SSD1306 OLED (PID 326), sharing the
GPIO6/GPIO10 I2C bus with the BME280:

```cpp
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include <Wire.h>

Adafruit_SSD1306 display(128, 64, &Wire, -1);

void setup() {
  Wire.begin(6, 10);
  if (!display.begin(SSD1306_SWITCHCAPVCC, 0x3C)) return;
  display.clearDisplay();
  display.setTextSize(1);
  display.setTextColor(SSD1306_WHITE);
  display.setCursor(0, 0);
  display.println("ESP32-C5");
  display.println("BME280 ready");
  display.display();
}

void loop() {}
```

Adafruit's [PID 326 product page](https://www.adafruit.com/product/326) showed
US$17.50, in stock, and Add to Cart on 2026-11-10. The page says I2C is the
default configuration and the interface can be selected by jumpers. The
committed firmware smoke uses raw I2C register writes and paints a checker
pattern; this Arduino library sketch was not compiled by that run.

### ESP32-S3-DevKitC-1 v1.1 N16R8 — ADC input

ESP32 remains demand rank **#1**. This extends the existing DevKitC board
coverage; it is not a new MCU or board pick. The S3 SENS model samples a
potentiometer on GPIO4 / ADC1_CH3. A centered 1650 mV input returns 2047 in the
local deterministic smoke. The register-level smoke evidence and current board
availability receipt are in
[`examples/esp32-s3-devkitc-1-n16r8/`](../../examples/esp32-s3-devkitc-1-n16r8/).

Copy-paste Arduino example for a raw 12-bit reading. Connect a 10 kΩ
potentiometer between 3V3 and GND, with the wiper on J1 pin 4 / GPIO4:

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  Serial.printf("GPIO4_ADC1_CH3=%d\n", analogRead(4));
}

void loop() {}
```

Espressif's [live DevKits catalog](https://www.espressif.com/products/devkits)
lists the exact N16R8 ordering SKU with a Mouser link. Its [current v1.1 board
guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
maps GPIO4 to ADC1_CH3 and marks older N8 and N8R2 SKUs EOL, not N16R8. The
dated receipt is in
[`evidence/adc/vendor-status.md`](../../examples/esp32-s3-devkitc-1-n16r8/evidence/adc/vendor-status.md).

### ESP32-S3-DevKitC-1 N16R8 — second I²C controller

This is maintenance/depth work on the existing ESP32 rank-1 board, not a new
board or MCU pick. It routes I²C1 through GPIO41/GPIO42 and reads the BME280
ID (`0x60`) and modeled raw temperature (`0x082DC0`). Copy-paste Arduino code,
wiring, local smoke logs, and the live vendor receipt are in the board
[README and evidence](../../examples/esp32-s3-devkitc-1-n16r8/README.md).

With an Adafruit BME280 at address `0x77`, on GPIO41 SDA and GPIO42 SCL:

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire1.begin(41, 42, 100000);
  if (bme.begin(0x77, &Wire1)) {
    Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
    Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
  }
}

void loop() {}
```

### ESP32-S3-DevKitC-1 N16R8 — SHT40 temperature and humidity

ESP32 remains demand rank **#1**. This is maintenance/depth work on the
existing DevKitC board: it attaches the buyable SHT40 breakout to I²C1 on
GPIO41/GPIO42 and adds a new environmental-sensor use case. The local firmware
checks the SHT40 serial response, the 25 °C / 50% RH modeled sample, and both
Sensirion CRC-8 bytes. It reuses the existing I²C1 and SHT40 models.

Wire an Adafruit SHT40 STEMMA QT breakout (PID 4885) to 3V3, GND, SDA=GPIO41,
and SCL=GPIO42. This copy-paste Arduino example uses Adafruit's SHT4x library:

~~~cpp
#include <Adafruit_SHT4x.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_SHT4x sht4;

void setup() {
  Serial.begin(115200);
  Wire1.begin(41, 42, 100000);
  if (!sht4.begin(&Wire1)) {
    Serial.println("SHT40 not found on I2C1");
    return;
  }
  sht4.setPrecision(SHT4X_HIGH_PRECISION);
  sensors_event_t humidity, temperature;
  if (sht4.getEvent(&humidity, &temperature)) {
    Serial.printf("Temperature: %.2f C, RH: %.2f %%\n",
                  temperature.temperature, humidity.relative_humidity);
  }
}

void loop() {}
~~~

The [current ESP32-S3-DevKitC-1 v1.1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
lists the N16R8 variant and documents GPIO41/GPIO42 on the J3 header. Mouser's
[exact N16R8 listing](https://www.mouser.com/ProductDetail/Espressif-Systems/ESP32-S3-DevKitC-1-N16R8)
showed 77 in stock on 2026-11-10. Adafruit's [SHT40 breakout page](https://www.adafruit.com/product/4885)
showed PID 4885 at $5.95 with Add to Cart available. The local firmware, smoke
logs, and dated availability receipt are in
[the SHT40 evidence directory](../../examples/esp32-s3-devkitc-1-n16r8/evidence/sht40/).
The hosted catalog does not contain an SHT40 entry, so this has no hosted
model_verified result.

## RP2040 / RP2350 — demand target #2

### Raspberry Pi Pico 2 / RP2350 (SC1631)

RP2040 is MCU-use rank **#5 at 12%** in the JetBrains survey. RP2350 is the
current flagship successor and is included in target #2, but was not measured
as a separate survey row. The Pico 2 was already onboarded before this round;
this is community-coverage maintenance and a refreshed local smoke record, not
a new MCU onboarding.

The narrow Pico 2 profile boots Cortex-M33 firmware, captures a UART0 marker,
and toggles the on-board LED at GP25 through SIO. The local smoke passed all
three assertions (`PICO2_SMOKE_OK`, at least eight GP25 edges, and the expected
step-limit stop). It does not exercise the optional Hazard3/RISC-V cores,
TrustZone, USB, or ROM boot. There is no physical-board capture. The external
hosted verifier has no Pico 2 target, so no hosted `labwired_verify` verdict is
claimed. The dated run and complete local artifacts are in
[`examples/pico2/evidence/community/`](../../examples/pico2/evidence/community/).

Copy-paste Arduino-Pico sketch for the Pico 2's on-board LED on GP25:

```cpp
void setup() {
  pinMode(25, OUTPUT);
}

void loop() {
  digitalWrite(25, HIGH);
  delay(250);
  digitalWrite(25, LOW);
  delay(250);
}
```

The committed simulation run uses the repository's bare-metal Rust firmware to
exercise the same GP25 output; this Arduino sketch itself was not compiled in
that run.

Raspberry Pi's [live Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
said **“Available now from $5”** and committed the Pico 2 series to production
through at least January 2040 when checked on 2026-11-10. Mouser's [exact SC1631
listing](https://www.mouser.com/en/c/embedded-solutions/?m=Raspberry+Pi&series=Pico+2)
showed 3,515 in stock at $5.00. The saved availability receipt is
[`vendor-status.md`](../../examples/pico2/evidence/community/vendor-status.md).

### ESP32-DevKitC-32E / ESP32-WROOM-32E

The board profile boots Arduino firmware and exercises UART0, UART1 TX, GPIO,
I2C0 and I2C1 with a BME280, SPI3 to an SSD1306 OLED, RMT output, ADC1 channel
0, DAC, LEDC PWM, PCNT, and MCPWM. The ADC smoke
checks `analogRead(36)` on GPIO36 / ADC1_CH0 with a potentiometer fixture; a
1650 mV input returns 2047 at 12-bit resolution. The LEDC smoke uses stock
`ledcAttach` / `ledcWrite` output on GPIO23 / J3 pin 2 for an external
SG90-class servo fixture. An earlier round added a BME280 to the second I2C
controller, routed to GPIO16/GPIO17 (J3 pins 12/11). The smoke reads BME280 ID
`0x60` and raw temperature `0x082DC0`. This is an ESP32 rank-1 coverage
extension on the existing board, not a new board or MCU demand pick.

This round extends the existing external microSD SPI profile on the same
rank-1 board. It uses VSPI/SPI3 on GPIO18/19/23 with CS=GPIO5, mounts a fixed
FAT16 image, reads `/README.TXT`, creates `/ROUND54.TXT`, and verifies the bytes
after writing and reopening it. GPIO16 marks a successful round trip. Single
sector writes persist during one simulation run; the seeded image resets on the
next run. This is ESP32 family coverage depth on an existing board, not a new
board/MCU demand pick. Live component availability is recorded in
[`microsd/vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/microsd/vendor-status.md).
The repository-local smoke record is in the evidence directory. The separate
hosted catalog's independent microSD model does not carry this local FAT16
image or its mutable sectors, so the repo-local smoke is the evidence for the
write/read behavior. See
[`hosted-verification.md`](../../examples/esp32-devkitc-32e/evidence/microsd/hosted-verification.md).

Copy-paste Arduino example for the Adafruit Micro SD SPI/SDIO breakout (PID
4682) and a microSD card that supports SPI mode. Wire 3V3/GND, CLK=GPIO18,
DO=GPIO19, CMD/SI=GPIO23, and CS=GPIO5:

```cpp
#include <Arduino.h>
#include <SD.h>
#include <SPI.h>

#include <string.h>

constexpr int CS_PIN = 5;

void setup() {
  Serial.begin(115200);
  pinMode(CS_PIN, OUTPUT);
  digitalWrite(CS_PIN, HIGH);
  SPI.begin(18, 19, 23, CS_PIN);
  if (!SD.begin(CS_PIN, SPI, 400000)) {
    Serial.println("microSD mount failed");
    return;
  }

  File file = SD.open("/README.TXT", FILE_READ);
  const char expected[] = "LabWired SD card demo.\r\n";
  char contents[sizeof(expected)] = {};
  size_t count = 0;
  if (file) {
    while (file.available() && count < sizeof(contents)) {
      contents[count++] = static_cast<char>(file.read());
    }
    file.close();
  }
  if (count == sizeof(expected) - 1 &&
      memcmp(contents, expected, sizeof(expected) - 1) == 0) {
    const char text[] = "round-trip write passed\r\n";
    File output = SD.open("/ROUND54.TXT", FILE_WRITE);
    size_t written = output
        ? output.write(reinterpret_cast<const uint8_t *>(text), sizeof(text) - 1)
        : 0;
    if (output) output.close();

    File input = SD.open("/ROUND54.TXT", FILE_READ);
    char readback[sizeof(text)] = {};
    size_t readback_count = 0;
    if (input) {
      while (input.available() && readback_count < sizeof(readback)) {
        readback[readback_count++] = static_cast<char>(input.read());
      }
      input.close();
    }
    if (written == sizeof(text) - 1 && readback_count == sizeof(text) - 1 &&
        memcmp(readback, text, sizeof(text) - 1) == 0) {
      Serial.println("microSD file write/read OK");
    } else {
      Serial.println("microSD file write/read failed");
    }
  } else {
    Serial.println("microSD file read failed");
  }
}

void loop() {}
```

Run the committed board smoke with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-microsd \
  --script examples/esp32-devkitc-32e/microsd-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-microsd.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/microsd \
  --firmware-build-script scripts/build-esp32-devkitc-32e-microsd-smoke.sh
```

Espressif's [current DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
lists the exact 32E board and maps J2 pin 3 / VP to GPIO36 / ADC1_CH0. The
[Mouser listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
reported immediate stock during the current check; DigiKey lists the SKU as
Active, though its stock view differed. Full lifecycle evidence is in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/adc/vendor-status.md).

Copy-paste Arduino example for a BME280 on I2C1, with SDA on GPIO16 and SCL on
GPIO17:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

void setup() {
  Serial.begin(115200);
  Wire1.begin(16, 17, 100000);
  Wire1.beginTransmission(kBme280Address);
  Wire1.write(0xD0);
  if (Wire1.endTransmission(false) != 0) return;
  if (Wire1.requestFrom(kBme280Address, static_cast<uint8_t>(1)) == 1 &&
      Wire1.read() == 0x60) {
    Serial.println("BME280 found on I2C1");
  }
}

void loop() {}
```

The committed simulator smoke drives I2C1's command registers directly. It
does not claim to verify the higher-level `Wire1` driver. Live board and sensor
availability evidence is in
[`i2c1-bme280/vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/i2c1-bme280/vendor-status.md).

Copy-paste Arduino example for a potentiometer wired from 3V3 to GND with its
wiper connected to J2 pin 3 / GPIO36:

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  const int raw = analogRead(36);  // ADC1_CH0 / VP
  Serial.printf("ADC1_CH0=%d\n", raw);
}

void loop() {}
```

Copy-paste Arduino example for a servo signal on J3 pin 2 / GPIO23. Power the
physical servo separately from 5 V and connect its ground to the board ground:

```cpp
#include <Arduino.h>

void setup() {
  constexpr uint8_t servo_pin = 23;
  if (!ledcAttach(servo_pin, 50, 14)) return;
  ledcWrite(servo_pin, 1187);  // about 1.45 ms per 49.92 Hz frame
}

void loop() {}
```

Run it against the committed board profile with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-ledc-servo \
  --script examples/esp32-devkitc-32e/ledc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-ledc-servo.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/ledc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-ledc-smoke.sh
```

The live board status and in-stock check is in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/ledc/vendor-status.md).
The smoke asserts the channel-0 duty shadow and GPIO23 edges. The SG90 twin is
not evidence of physical servo torque, current draw, or mechanical motion.

Copy-paste Arduino example for a UART sensor or module whose RX connects to J3
pin 12 / GPIO16 (connect grounds; this profile tests UART1 TX only):

```cpp
#include <Arduino.h>

void setup() {
  Serial1.begin(115200, SERIAL_8N1, -1, 16);  // RX unused, TX=GPIO16
  Serial1.println("sensor request");
}

void loop() {}
```

The smoke sends repeated `U` bytes and checks GPIO16 edges. The exact board
availability check for this round is in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/uart1/vendor-status.md).

This round adds stock Arduino VSPI/SPI3 output to a 128x64 SSD1306 OLED on
SCK=GPIO18, MISO=GPIO19, MOSI=GPIO23, CS=GPIO5, and D/C=GPIO17. The local
smoke runs the initialization sequence and asserts that the display model's
top 8-pixel page is fully lit. The hosted verifier separately observes at
least 16 GPIO18 clock transitions. This is master TX coverage; MISO slave
responses and the panel reset input are not modeled.

Copy-paste Arduino example for an Adafruit 128x64 SSD1306 breakout configured
for 4-wire SPI (Adafruit PID 938; cut its two interface jumpers from I2C):

```cpp
#include <Arduino.h>
#include <SPI.h>

constexpr int kCs = 5;
constexpr int kDc = 17;

void setup() {
  Serial.begin(115200);
  pinMode(kCs, OUTPUT);
  pinMode(kDc, OUTPUT);
  digitalWrite(kCs, HIGH);
  digitalWrite(kDc, LOW);
  SPI.begin(18, 19, 23, kCs);  // SCK, MISO, MOSI, CS
  SPI.beginTransaction(SPISettings(1000000, MSBFIRST, SPI_MODE0));

  const uint8_t init[] = {0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40,
                          0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xDA, 0x12,
                          0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6,
                          0xAF, 0x21, 0x00, 0x7F, 0x22, 0x00, 0x00};
  digitalWrite(kCs, LOW);
  for (uint8_t byte : init) SPI.transfer(byte);
  digitalWrite(kDc, HIGH);
  for (int i = 0; i < 128; ++i) SPI.transfer(0xFF);  // top 8-pixel page
  digitalWrite(kCs, HIGH);
  SPI.endTransaction();
}

void loop() {}
```

The exact board and display availability check is recorded in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/spi-oled/vendor-status.md).
The display framebuffer assertion, hosted SPI clock oracle, smoke logs, and
scoreboard are in
[`evidence/spi-oled/`](../../examples/esp32-devkitc-32e/evidence/spi-oled/)
and [`onboarding-scoreboard.md`](../../examples/esp32-devkitc-32e/evidence/onboarding-scoreboard.md).

Run it against the committed board profile with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-adc \
  --script examples/esp32-devkitc-32e/adc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-adc.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/adc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-adc-smoke.sh
```

The run log is [`run_smoke.log`](../../examples/esp32-devkitc-32e/evidence/adc/logs/run_smoke.log);
the complete board scope and other copy-paste examples are in the
[DevKitC-32E README](../../examples/esp32-devkitc-32e/README.md).

The deterministic ADC fixture models an injected 1650 mV level. It does not
model ADC noise, calibration, or the analog attenuation curve, and the
potentiometer is an external fixture rather than an onboard board component.

This round adds coverage for the existing classic ESP32 PCNT model in the exact
DevKitC profile. The local smoke counts eight rising edges from GPIO23 / J3 pin
2 through the GPIO input matrix. GPIO16 / J3 pin 12 is set high only when PCNT reads eight; the
hosted verifier asserts that output. ESP32 is rank 1; this is low-cost coverage
integration of an existing model on the same board, not a new board/MCU demand
pick. It does not model the PCNT glitch filter, encoder decoding,
or an external encoder/pulse sensor. Board and module availability was checked
live; see [`PCNT vendor status`](../../examples/esp32-devkitc-32e/evidence/pcnt/vendor-status.md).

Copy-paste Arduino example for eight GPIO23 pulses counted by PCNT unit 0:

```cpp
#include <Arduino.h>
#include "driver/pcnt.h"

constexpr gpio_num_t kPulsePin = GPIO_NUM_23;
constexpr gpio_num_t kPassPin = GPIO_NUM_16;

void setup() {
  pcnt_config_t config = {};
  config.pulse_gpio_num = kPulsePin;
  config.ctrl_gpio_num = PCNT_PIN_NOT_USED;
  config.channel = PCNT_CHANNEL_0;
  config.unit = PCNT_UNIT_0;
  config.pos_mode = PCNT_COUNT_INC;
  config.neg_mode = PCNT_COUNT_DIS;
  config.lctrl_mode = PCNT_MODE_KEEP;
  config.hctrl_mode = PCNT_MODE_KEEP;
  config.counter_h_lim = 1000;
  config.counter_l_lim = -1000;

  pinMode(static_cast<uint8_t>(kPassPin), OUTPUT);
  digitalWrite(static_cast<uint8_t>(kPassPin), LOW);
  if (pcnt_unit_config(&config) != ESP_OK) return;

  // The driver configures the pulse input, so re-enable its output afterward.
  pinMode(static_cast<uint8_t>(kPulsePin), OUTPUT);
  digitalWrite(static_cast<uint8_t>(kPulsePin), LOW);
  pcnt_counter_pause(PCNT_UNIT_0);
  pcnt_counter_clear(PCNT_UNIT_0);
  pcnt_counter_resume(PCNT_UNIT_0);

  for (int i = 0; i < 8; ++i) {
    digitalWrite(static_cast<uint8_t>(kPulsePin), HIGH);
    delayMicroseconds(100);
    digitalWrite(static_cast<uint8_t>(kPulsePin), LOW);
    delayMicroseconds(100);
  }

  int16_t count = 0;
  if (pcnt_get_counter_value(PCNT_UNIT_0, &count) == ESP_OK && count == 8) {
    digitalWrite(static_cast<uint8_t>(kPassPin), HIGH);
  }
}

void loop() {}
```

This round adds a smoke for MCPWM0 operator A to the same rank-1 DevKitC-32E
profile. It uses Espressif's legacy driver to set a 50 Hz, 7.5% center-servo
command and checks the compare-A register. The attached servo twin follows the
MCPWM duty observer; pad-level waveform generation and physical servo loading
are not modeled. The local smoke passed all three assertions; it does not read
servo-angle telemetry. This is a coverage extension to a supported board, not
a new MCU or vendor.

Copy-paste Arduino example for an SG90-class servo signal on J3 pin 2 / GPIO23.
Power a physical servo separately from 5 V and share ground with the board:

```cpp
#include <Arduino.h>
#include "driver/mcpwm.h"
#include "driver/gpio.h"

void setup() {
  constexpr gpio_num_t signal = GPIO_NUM_23;
  if (mcpwm_gpio_init(MCPWM_UNIT_0, MCPWM0A, signal) != ESP_OK) return;

  mcpwm_config_t config = {};
  config.frequency = 50;
  config.cmpr_a = 0;
  config.cmpr_b = 0;
  config.counter_mode = MCPWM_UP_COUNTER;
  config.duty_mode = MCPWM_DUTY_MODE_0;
  if (mcpwm_init(MCPWM_UNIT_0, MCPWM_TIMER_0, &config) != ESP_OK) return;

  if (mcpwm_set_duty(MCPWM_UNIT_0, MCPWM_TIMER_0, MCPWM_OPR_A, 7.5f) != ESP_OK) {
    return;
  }
}

void loop() {}
```

Reproduce with the committed local profile from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-mcpwm \
  --script examples/esp32-devkitc-32e/mcpwm-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-mcpwm-servo.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/mcpwm \
  --firmware-build-script scripts/build-esp32-devkitc-32e-mcpwm-smoke.sh
```

The current lifecycle receipt and smoke artifacts are linked from the
[`DevKitC guide`](../../examples/esp32-devkitc-32e/README.md).
The connected hosted verifier could not execute this sketch because its
connector schema rejects Arduino even though the ESP32 target lists Arduino as
supported; the local result is not a physical-board run.

## Remaining demand-ranked entries

Before this page is ready for publication, add reviewed coverage and a
copy-paste example for the supported Arduino AVR path, STM32F103, and current
Nucleo/Discovery boards. WCH CH32V003 already has a local booting smoke and a
copy-paste example in [`docs/community/hardware-coverage-draft.md`](../community/hardware-coverage-draft.md);
consolidate that entry here before publication. Each entry must link to its
exact board or part evidence and clearly identify any lifecycle status.
