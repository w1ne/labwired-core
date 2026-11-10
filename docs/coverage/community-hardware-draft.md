# Community hardware quickstarts — draft

This is a review draft for the community coverage page. It is intentionally not
listed in `mkdocs.yml` and has not been published. The current draft covers the
ESP32 family entry; additional demand-ranked entries need to be added and
reviewed before publication.

## ESP32 — rank 1, 38% of reported MCU use

### ESP32-DevKitC-32E / ESP32-WROOM-32E

The board profile boots Arduino firmware and exercises UART0, UART1 TX, GPIO,
I2C with a BME280, SPI3 to an SSD1306 OLED, RMT output, ADC1 channel 0, DAC,
and LEDC PWM. The ADC smoke
checks `analogRead(36)` on GPIO36 / ADC1_CH0 with a potentiometer fixture; a
1650 mV input returns 2047 at 12-bit resolution. The LEDC smoke uses stock
`ledcAttach` / `ledcWrite` output on GPIO23 / J3 pin 2 for an external
SG90-class servo fixture. This round adds stock `Serial1` TX routed to GPIO16 /
J3 pin 12 at 115200 baud; it sends `UUUUUUUU` and the smoke requires signal
edges on that header pin.

Espressif's [current DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
lists the exact 32E board and maps J2 pin 3 / VP to GPIO36 / ADC1_CH0. The
[Mouser listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
reported immediate stock during the current check; DigiKey lists the SKU as
Active, though its stock view differed. Full lifecycle evidence is in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/adc/vendor-status.md).

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
copy-paste example for RP2040/RP2350, the supported Arduino AVR path, STM32F103
and current Nucleo/Discovery boards, and the WCH CH32V003 community target. Each
entry must link to its exact board or part evidence and clearly identify any
legacy lifecycle status.
