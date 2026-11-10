# ESP32-DevKitC-32E GPIO, PCNT, UART, BME280 I2C, SPI OLED, microSD, RMT, ADC, DAC, LEDC, and MCPWM smokes

**Demand rank:** ESP32 is rank 1 in the standing list (38% of reported MCU
use). An earlier round added an I2C1 BME280 profile on GPIO16/GPIO17. This
round extends the SPI microSD model on VSPI to mount a FAT16 demo volume, read
`/README.TXT`, and write/read back `/ROUND54.TXT`. This is coverage depth on
the same customer board, not a new board/MCU demand pick. Earlier rounds also
added PCNT unit 0, the stock Arduino VSPI path
to a 128x64 SSD1306 display, and MCPWM0 servo control.

## Board and modeled scope

The board is Espressif's exact **ESP32-DevKitC-32E** with an ESP32-WROOM-32E
module and 4 MB flash. The narrow board profile exposes classic ESP32 GPIO,
PCNT unit 0, UART0 console, UART1 TX routed to GPIO16, I2C0 and I2C1, VSPI/SPI3,
ADC1 channel 0, DAC1/2, LEDC, MCPWM0 operator A, and RMT TX.
GPIO23 maps to J3 pin 2; it is a header pin, not a built-in LED. For a physical
blink, connect an LED in series with a resistor from GPIO23 to GND.

| Board connection | MCU behavior | Smoke |
| --- | --- | --- |
| J3 pin 2 | GPIO23 | Base smoke toggles the output and requires at least 16 edges |
| J3 pin 2 | PCNT unit 0, channel 0 | Counts eight rising edges routed from GPIO23; GPIO16 goes high on the exact count |
| UART0 output | Serial console capture | Base smoke asserts `ESP32_DEVKITC32E_UART0_OK` |
| J3 pin 12 | UART1 TX routed to GPIO16 | UART1 smoke sends `UUUUUUUU` at 115200 baud and requires GPIO16 edges |
| J2 GPIO21/GPIO22 | I2C0 SDA/SCL | BME280 smoke reads sensor ID and temperature ADC data |
| J3 GPIO16/GPIO17 | I2C1 SDA/SCL | I2C1 smoke reads BME280 ID `0x60` and raw temperature `0x082DC0` |
| J3 GPIO18/19/23/5, GPIO17 | VSPI/SPI3 clock, MISO, MOSI, CS, and OLED D/C | SPI OLED smoke sends SSD1306 init commands and paints the first 8-row page |
| J3 GPIO18/19/23/5, GPIO17, GPIO16 | VSPI/SPI3 microSD SCK, MISO, MOSI, CS; mount and file-write/read pass outputs | microSD smoke reads `/README.TXT`, writes `/ROUND54.TXT`, and verifies its contents after reopening |
| J2 pin 3 / VP (GPIO36) | ADC1 channel 0 | ADC smoke reads a 1650 mV potentiometer through `analogRead(36)` and expects 2047 at 12-bit resolution |
| J3 pin 2 (GPIO23) | LEDC high-speed channel 0 PWM output | LEDC smoke runs 50 Hz, 14-bit PWM to an external SG90-class servo twin and requires committed duty plus GPIO edges |
| J3 pin 2 (GPIO23) | MCPWM0 operator A duty observer | MCPWM smoke sets a 50 Hz, 7.5% center command and checks the compare-A register; the servo twin follows duty, while pad-level MCPWM waveforms are outside the model |
| GPIO23 header | RMT channel 0 TX matrix output | RMT smoke sends red to an external WS2812 pixel and waits for TX_END |
| GPIO18 header | RMT channel 1 TX matrix output | RMT smoke sends green to a second pixel and waits for TX_END |
| J2 pin 9 (GPIO25) | DAC1 direct 8-bit analog output | DAC smoke calls `dacWrite(25, 128)` and checks the DAC code and enable bit |

The BME280 breakout uses the standard `0x77` address and is powered directly
from board 3V3/GND. Firmware reads ID register `0xD0` (`0x60`) and the raw
temperature registers `0xFA..0xFC` (`0x082DC0` in the deterministic model).
The breakout, rather than a built-in board component, is declared as an
external device in the BME280 system manifest.

The ADC test uses the board's J2 pin 3 (VP), which Espressif maps to GPIO36 /
ADC1_CH0. Its potentiometer is an external simulation fixture, not a component
fitted to the board. The default midpoint supplies 1650 mV and produces code
2047 in the deterministic 12-bit model. The ADC model does not include noise,
calibration, or the analog attenuation curve, so this is a simulator input
check rather than a physical accuracy claim.

The LEDC profile uses GPIO23 / J3 pin 2, also used by the separate GPIO and
RMT profiles. `ledcAttach(23, 50, 14)` plus `ledcWrite(23, 1187)` produces a
near-50 Hz frame with a roughly 1.45 ms command pulse in the model. The system
attaches an external SG90 fixture; the smoke asserts the channel-0 duty shadow
and GPIO23 edges, not servo angle telemetry. Power a physical servo from a
separate 5 V supply and share ground with the board. The twin does not model
servo current, torque, gearing, or mechanical load.

The MCPWM profile uses MCPWM0 timer 0 / operator A at 50 Hz with a 7.5% duty
command (about 1.5 ms of a 20 ms frame). The real register model derives the
timer frequency and duty and notifies the external SG90 fixture, which tracks
the commanded angle. It does not synthesize MCPWM pad edges or model actuator
current, torque, gearing, or mechanical load. The firmware programs MCPWM only
while LEDC remains idle; the acceptance checks the compare register and marker,
not servo-angle telemetry.

Copy-paste Arduino example for an SG90-class servo signal on GPIO23 / J3 pin 2.
Power a physical servo separately from 5 V and connect its ground to the board:

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

The RMT model implements per-channel TX control/status offsets, RMTMEM symbol
RAM, independent TX timing for channels 0 and 1, GPIO-matrix output routing,
and per-channel TX_END after the stream's reset interval. It does not model RX,
or advancing APB FIFO pointers. This smoke exercises channels 0 and 1; channels
2–7 use the same transmit path but do not have a smoke here. The pixels are
external generic WS2812-compatible fixtures, not onboard DevKitC LEDs. The
UART0 and UART1 TX controllers are modeled. UART1 RX and the board's
USB-to-UART bridge are outside this profile; the UART1 test proves routed TX
edges on GPIO16, while console capture does not prove USB serial behavior.
The SPI profile covers VSPI/SPI3 master TX to an external 128x64 SSD1306 SPI
breakout on SCK=GPIO18, MOSI=GPIO23, CS=GPIO5, and D/C=GPIO17; MISO is exposed
on the header but unused by this display. The smoke asserts a fully inked top
8-pixel page after stock Arduino `SPI` transfers. The panel reset input is not
modeled; the selected breakout has an auto-reset circuit. This does not verify
an external SPI slave's MISO path. The hosted verifier separately asserts at
least 16 GPIO18 clock transitions; its catalog lacks the SPI OLED, so the
local display-region assertion is the panel-model evidence. Details and the
hosted rail assumption are in
[`evidence/spi-oled/hosted-verification.md`](evidence/spi-oled/hosted-verification.md).

The PCNT acceptance firmware is [`firmware/src/pcnt_smoke.cpp`](firmware/src/pcnt_smoke.cpp).
It configures PCNT unit 0 channel 0 for rising-edge increments on GPIO23,
generates eight 100 µs high/low pulses, and reads the counter. GPIO16 is driven
high only when the count is exactly eight. The local smoke checks the count
marker, GPIO23 pulse edges, and GPIO16 pass edge. The hosted verifier checks
GPIO16 high as its behavioral oracle. The PCNT glitch filter is not modeled;
external pulse sources, encoder decoding, and other units/channels are outside
this smoke.

Wi-Fi/Bluetooth, timers, flash
boot behavior, and the 5 V power LED are outside these profiles. The LEDC
profile does not claim general-purpose timer coverage. The base ESP32 descriptor
still has zero-answering system stubs for I2S0, I2S1, and UHCI0; this board
profile does not claim those peripherals.

The DAC profile covers static 8-bit direct output on GPIO25/DAC1 and GPIO26/DAC2.
The model scales the code against a 3.3 V reference and exposes the voltage
through its analog output probe; code 128 is 1656 mV. DMA stream output and the
cosine generator remain outside this profile. The firmware smoke programs DAC1
with Arduino's `dacWrite` API and checks its RTC_IO DAC register bits. A focused
model unit test asserts voltage scaling and disabled-channel behavior.

## Copy-paste firmware example

This complete Arduino sketch reads the chip ID and raw temperature registers
from an Adafruit BME280 breakout on GPIO21/GPIO22:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

bool readRegisters(uint8_t first, uint8_t *bytes, uint8_t count) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, count) != count) return false;
  for (uint8_t i = 0; i < count; ++i) bytes[i] = Wire.read();
  return true;
}

bool writeRegister(uint8_t reg, uint8_t value) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(21, 22, 100000);  // SDA=GPIO21, SCL=GPIO22

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1) || id != 0x60) {
    Serial.println("BME280 not found");
    return;
  }

  // Enable humidity first, then temperature/pressure oversampling and normal mode.
  if (!writeRegister(0xF2, 0x01) || !writeRegister(0xF4, 0x27)) {
    Serial.println("BME280 setup failed");
    return;
  }
  delay(100);

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280 read failed");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  Serial.printf("BME280_TEMP_RAW=0x%06lX\n", static_cast<unsigned long>(raw));
}

void loop() {}
```

The local acceptance firmware is
[`firmware/src/bme280_smoke.cpp`](firmware/src/bme280_smoke.cpp). It programs
the classic ESP32 I2C0 command registers directly and reads the same ID and
temperature registers. The Arduino ESP32 3.3.12 `Wire` driver stalls after its
first split-command `END_DETECT` boundary in the current simulator, so the
local smoke does not claim to verify that driver path.

For the second bus, connect BME280 SDA to GPIO16 / J3 pin 12 and SCL to
GPIO17 / J3 pin 11. On physical hardware, the Arduino API is `Wire1`:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

void setup() {
  Serial.begin(115200);
  Wire1.begin(16, 17, 100000);  // SDA=GPIO16, SCL=GPIO17
  Wire1.beginTransmission(kBme280Address);
  Wire1.write(0xD0);            // BME280 chip ID register
  if (Wire1.endTransmission(false) != 0) return;
  if (Wire1.requestFrom(kBme280Address, static_cast<uint8_t>(1)) == 1 &&
      Wire1.read() == 0x60) {
    Serial.println("BME280 found on I2C1");
  }
}

void loop() {}
```

The local I2C1 smoke is
[`firmware/src/bme280_i2c1_smoke.cpp`](firmware/src/bme280_i2c1_smoke.cpp).
It drives the second controller's command-list registers directly and reads
ID `0x60` plus raw temperature `0x082DC0`. The sketch above documents the
Arduino driver usage; the smoke does not claim that `Wire1` itself is verified
in the current simulator.

The ADC acceptance firmware is [`firmware/src/adc_smoke.cpp`](firmware/src/adc_smoke.cpp).
It calls the Arduino `analogRead(36)` API and checks the resulting ADC1_CH0
value against the potentiometer fixture. The run log and metrics are committed
under [`evidence/adc/`](evidence/adc/).

The RMT smoke firmware is [`firmware/src/rmt_smoke.cpp`](firmware/src/rmt_smoke.cpp).
It writes separate red and green GRB frames into channel 0 and 1 RMTMEM regions,
starts both channels, and waits for both timed TX_END bits. External pixel
models independently decode GPIO23 and GPIO18, and the smoke requires both
colors.

The LEDC servo acceptance firmware is
[`firmware/src/ledc_servo_smoke.cpp`](firmware/src/ledc_servo_smoke.cpp). It
uses the Arduino 3.x `ledcAttach` and `ledcWrite` APIs on GPIO23. The committed
smoke requires the channel-0 duty register value, at least two GPIO23 edges,
and the success marker printed after the PWM setup returns successfully. A
separate hosted `labwired_verify` result checks the same duty register and at
least two GPIO23 edges; it does not assert servo angle.

The UART1 acceptance firmware is
[`firmware/src/uart1_smoke.cpp`](firmware/src/uart1_smoke.cpp). It remaps the
UART1 TX output to GPIO16 / J3 pin 12 and sends repeated `U` bytes. The local
test checks the console marker and at least 32 GPIO16 edges; the hosted
verifier separately model-verifies GPIO16 transitions.

The MCPWM acceptance firmware is
[`firmware/src/mcpwm_smoke.cpp`](firmware/src/mcpwm_smoke.cpp). It uses the
ESP-IDF legacy `mcpwm_gpio_init`, `mcpwm_init`, and `mcpwm_set_duty` APIs.
The local smoke requires the success marker and checks MCPWM0's operator-A
compare register equals 1500. The connected hosted verifier could not run this
board's sketch because its runtime accepts Arduino while its connector schema
rejects that language; details are recorded in
[`evidence/mcpwm/hosted-verification.md`](evidence/mcpwm/hosted-verification.md).

## Reproduce

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-bme280 \
  --script examples/esp32-devkitc-32e/bme280-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-bme280.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/bme280 \
  --firmware-build-script scripts/build-esp32-devkitc-32e-bme280.sh
```

The build log, local simulator output, and generated metrics are committed
under `evidence/bme280/`. This is deterministic digital-twin evidence, not a
physical-board run. The classic ESP32 hosted verifier does not support its
serial oracle; the committed local CLI smoke is the acceptance evidence.

Run the second-controller I2C1 smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-i2c1-bme280 \
  --script examples/esp32-devkitc-32e/i2c1-bme280-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-i2c1-bme280.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/i2c1-bme280 \
  --firmware-build-script scripts/build-esp32-devkitc-32e-i2c1-bme280-smoke.sh
```

The local smoke reads BME280 ID `0x60` and raw temperature `0x082DC0` through
I2C1. It is controller-level simulator evidence, not a physical-board run or
verification of the Arduino `Wire1` driver. The live lifecycle and availability
record is in [`evidence/i2c1-bme280/vendor-status.md`](evidence/i2c1-bme280/vendor-status.md).

Run the LEDC servo smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-ledc-servo \
  --script examples/esp32-devkitc-32e/ledc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-ledc-servo.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/ledc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-ledc-smoke.sh
```

The current board availability check for this profile is in
[`evidence/ledc/vendor-status.md`](evidence/ledc/vendor-status.md).

Run the PCNT pulse-count smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-pcnt \
  --script examples/esp32-devkitc-32e/pcnt-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-pcnt.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/pcnt \
  --firmware-build-script scripts/build-esp32-devkitc-32e-pcnt-smoke.sh
```

This smoke is a GPIO23-to-PCNT input-matrix self-loop. It does not model an
external encoder or pulse sensor. The local logs and hosted GPIO oracle are in
[`evidence/pcnt/`](evidence/pcnt/), and live availability is in
[`evidence/pcnt/vendor-status.md`](evidence/pcnt/vendor-status.md).

Run the MCPWM servo smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-mcpwm \
  --script examples/esp32-devkitc-32e/mcpwm-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-mcpwm-servo.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/mcpwm \
  --firmware-build-script scripts/build-esp32-devkitc-32e-mcpwm-smoke.sh
```

The committed run log, structured result, metrics, and live status receipt are
under [`evidence/mcpwm/`](evidence/mcpwm/). This is local digital-twin
evidence, not a physical-board test.

Run the RMT + pixel smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-rmt-neopixel \
  --script examples/esp32-devkitc-32e/rmt-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-rmt-neopixel.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/rmt-neopixel \
  --firmware-build-script scripts/build-esp32-devkitc-32e-rmt-smoke.sh
```

The run evidence belongs under `evidence/rmt-neopixel/`. This is deterministic
digital-twin evidence, not a physical-board run.

Run the ADC input smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-adc \
  --script examples/esp32-devkitc-32e/adc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-adc.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/adc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-adc-smoke.sh
```

Copy-paste Arduino example for a midpoint potentiometer wired from 3V3 to GND
with its wiper on J2 pin 3 / GPIO36:

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

The current lifecycle and stock check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md). The evidence records
why the exact board and WROOM-32E variant satisfy the no-obsolete-parts rule.
The current-round ADC availability and GPIO36 source check is recorded in
[`evidence/adc/vendor-status.md`](evidence/adc/vendor-status.md).

Run the DAC output smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-dac \
  --script examples/esp32-devkitc-32e/dac-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-dac.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/dac \
  --firmware-build-script scripts/build-esp32-devkitc-32e-dac-smoke.sh
```

Copy-paste Arduino example for a fixed midpoint on DAC1 (GPIO25 / J2 pin 9):

```cpp
#include <Arduino.h>

void setup() {
  dacWrite(25, 128);  // Nominal output: 1.656 V at a 3.3 V reference.
}

void loop() {}
```

GPIO26 / DAC2 is on J2 pin 10. The model drives the selected pin only when
firmware enables the DAC channel. Smoke logs and model-verification evidence
are committed under `evidence/dac/`.

Run the UART1 TX smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-uart1 \
  --script examples/esp32-devkitc-32e/uart1-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-uart1.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/uart1 \
  --firmware-build-script scripts/build-esp32-devkitc-32e-uart1-smoke.sh
```

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

The model covers UART1 TX output routed to GPIO16. It does not model an
external UART receiver or UART1 RX. The exact board availability check is in
[`evidence/uart1/vendor-status.md`](evidence/uart1/vendor-status.md).
Hosted oracle details, including the non-blocking assumed-power gap, are in
[`evidence/uart1/hosted-verification.md`](evidence/uart1/hosted-verification.md).

Run the SPI OLED smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-spi-oled \
  --script examples/esp32-devkitc-32e/spi-oled-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-spi-oled.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/spi-oled \
  --firmware-build-script scripts/build-esp32-devkitc-32e-spi-oled-smoke.sh
```

Copy-paste Arduino example for an Adafruit 128x64 SSD1306 breakout configured
for 4-wire SPI (CS=GPIO5, D/C=GPIO17, SCK=GPIO18, MOSI=GPIO23):

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
  SPI.begin(18, 19, 23, kCs);
  SPI.beginTransaction(SPISettings(1000000, MSBFIRST, SPI_MODE0));

  const uint8_t init[] = {0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40,
                          0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xDA, 0x12,
                          0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6,
                          0xAF, 0x21, 0x00, 0x7F, 0x22, 0x00, 0x00};
  digitalWrite(kDc, LOW);
  digitalWrite(kCs, LOW);
  for (uint8_t byte : init) SPI.transfer(byte);

  digitalWrite(kDc, HIGH);
  for (int i = 0; i < 128; ++i) SPI.transfer(0xFF);  // Solid top 8-pixel page
  digitalWrite(kCs, HIGH);
  SPI.endTransaction();
  Serial.println("OLED page painted");
}

void loop() {}
```

Connect the breakout to 3V3/GND and the listed signal pins; MISO is unused.
Adafruit PID 938 defaults to I2C, so cut its two interface jumpers to select SPI. The
current exact-board and display availability check is in
[`evidence/spi-oled/vendor-status.md`](evidence/spi-oled/vendor-status.md).
The smoke log and deterministic local display assertion are committed under
[`evidence/spi-oled/`](evidence/spi-oled/).

Run the microSD filesystem write/read smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-microsd \
  --script examples/esp32-devkitc-32e/microsd-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-microsd.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/microsd \
  --firmware-build-script scripts/build-esp32-devkitc-32e-microsd-smoke.sh
```

Copy-paste Arduino example for an Adafruit Micro SD SPI/SDIO breakout (PID
4682) and a microSD card supporting SPI mode. Connect 3V3/GND, CLK to GPIO18,
DO to GPIO19, CMD/SI to GPIO23, and CS to GPIO5. This example checks the seeded
file, creates `/ROUND54.TXT`, writes a marker, and reads the file back. GPIO16
is an optional pass output:

```cpp
#include <Arduino.h>
#include <SD.h>
#include <SPI.h>

#include <string.h>

constexpr int kCs = 5;
constexpr int kPass = 16;
constexpr char kWriteText[] = "round-trip write passed\r\n";

void setup() {
  Serial.begin(115200);
  pinMode(kCs, OUTPUT); pinMode(kPass, OUTPUT);
  digitalWrite(kCs, HIGH);
  digitalWrite(kPass, LOW);
  SPI.begin(18, 19, 23, kCs);
  if (!SD.begin(kCs, SPI, 400000)) {
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
  const bool initialReadOk = count == sizeof(expected) - 1 &&
      memcmp(contents, expected, sizeof(expected) - 1) == 0;

  File output = SD.open("/ROUND54.TXT", FILE_WRITE);
  size_t written = output
      ? output.write(reinterpret_cast<const uint8_t *>(kWriteText),
                     sizeof(kWriteText) - 1)
      : 0;
  if (output) output.close();

  File input = SD.open("/ROUND54.TXT", FILE_READ);
  char readback[sizeof(kWriteText)] = {};
  size_t readbackCount = 0;
  if (input) {
    while (input.available() && readbackCount < sizeof(readback)) {
      readback[readbackCount++] = static_cast<char>(input.read());
    }
    input.close();
  }
  const bool roundTripOk = written == sizeof(kWriteText) - 1 &&
      readbackCount == sizeof(kWriteText) - 1 &&
      memcmp(readback, kWriteText, sizeof(kWriteText) - 1) == 0;

  if (initialReadOk && roundTripOk) {
    digitalWrite(kPass, HIGH);
    Serial.println("microSD file write/read OK");
  } else {
    Serial.println("microSD file write/read failed");
  }
}

void loop() {}
```

The simulation card starts from a fixed 32 MiB SDHC image with a FAT16 volume
and `/README.TXT`. It supports CSD reads and single-block reads/writes; writes
are retained in memory for that simulation run and the seeded image is restored
for the next run. This is a deterministic profile, not a raw-flash or wear
model. A physical microSD card is not included with the breakout. The
committed local smoke and hosted initialization oracle evidence are in
[`evidence/microsd/`](evidence/microsd/); live board and breakout availability
is recorded in [`evidence/microsd/vendor-status.md`](evidence/microsd/vendor-status.md).
