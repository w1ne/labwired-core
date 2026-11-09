# ESP32-C61-DevKitC-1 v2.0 (N8R2): BME280, RGB LED, and LEDC servo

**Demand rank:** ESP32 family is rank 1 (38% MCU use; 42% of kits). ESP32-C61
has no separate survey rank. This round extends the existing C61 board with a
modeled LEDC servo path; it is board-peripheral maintenance within the highest
demand family, not a new chip or vendor pick.

The board profile runs bare-metal firmware built for `riscv32imc` (an
instruction subset of the C61's RV32IMAC core). The original smoke reads a
BME280 ID and raw temperature over I2C0 on GPIO4/GPIO5 and lights the onboard
WS2812B RGB LED on GPIO8. A second smoke configures LEDC channel 0 and routes a
49.8 Hz PWM signal to GPIO6 for an attached SG90 model. The board guide maps
GPIO4/5/6 to J1 pins 3/4/5 and connects UART0 to the USB-UART bridge; the
official schematic identifies the GPIO8 LED as D6 WS2812B.

## BME280 wiring

Connect an Adafruit BME280 breakout to 3V3 and GND, SDA to GPIO4 (J1 pin 3),
and SCL to GPIO5 (J1 pin 4). The configured address is `0x77`; set the
breakout's address jumper accordingly. The simulated system also keeps the
onboard RGB LED on GPIO8.

## LEDC servo wiring and Arduino example

Connect only the servo signal lead to GPIO6 (J1 pin 5). Power an SG90 from a
separate regulated 5 V supply and connect the supply ground to board ground;
do not power it from the GPIO pin. The C61 LEDC profile uses the reset-selected
40 MHz XTAL source, timer 2 at 14-bit resolution, and a nominal 49.8 Hz frame.

With Arduino-ESP32 3.x built as an ESP-IDF component for C61, the default XTAL
clock and pin-based LEDC API can set the midpoint duty:

```cpp
#include <Arduino.h>

constexpr uint8_t SERVO_PIN = 6;

void setup() {
  if (!ledcAttach(SERVO_PIN, 50, 14)) {
    while (true) {}
  }

  // 1187 / 16384 of a 20 ms frame is approximately a 1.45 ms pulse.
  if (!ledcWrite(SERVO_PIN, 1187)) {
    while (true) {}
  }
}

void loop() {}
```

## Arduino sketch example

The official [Arduino-ESP32 core supports C61](https://github.com/espressif/arduino-esp32/blob/master/docs/en/getting_started.rst), but using it currently requires an ESP-IDF component build or rebuilt core libraries; it is not a stock Arduino IDE board package. With the Adafruit BME280 and NeoPixel libraries installed, this sketch reads the sensor and sets the onboard pixel red:

```cpp
#include <Adafruit_NeoPixel.h>
#include <Adafruit_BME280.h>
#include <Wire.h>

Adafruit_NeoPixel rgb(1, 8, NEO_GRB + NEO_KHZ800);
Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_C61_UART0_OK");
  Wire.begin(4, 5);
  if (bme.begin(0x77, &Wire)) {
    Serial.println("BME280_ID=0x60");
    Serial.print("BME280_TEMP_C=");
    Serial.println(bme.readTemperature());
  } else {
    Serial.println("BME280_I2C_ERROR");
  }
  rgb.begin();
  rgb.setPixelColor(0, rgb.Color(255, 0, 0));
  rgb.show();
}

void loop() {}
```

## Run the committed smoke

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc \
  --crate firmware-esp32c61-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc/onboarding-smoke.yaml \
  --system configs/systems/esp32c61-devkitc.yaml \
  --out-dir examples/esp32c61-devkitc/evidence
```

The model smoke loads an ELF directly; ROM and second-stage boot are outside
this profile. Committed build and simulator evidence is under
[`evidence/`](evidence/), and live vendor availability is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The LEDC servo smoke uses a second firmware binary from the same package:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc-ledc-servo \
  --crate firmware-esp32c61-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc/ledc-servo-smoke.yaml \
  --system configs/systems/esp32c61-devkitc.yaml \
  --out-dir examples/esp32c61-devkitc/evidence/ledc-servo
```

The 2026-11-09 local run passed **6/6 assertions** at 1,000,000 steps. UART
reported `BME280_ID=0x60` and `BME280_TEMP_RAW=0x082DC0`; the GPIO8 RGB
waveform decoded as red.

The 2026-11-09 LEDC run passed **4/4 assertions** at 4,000,000 steps. It
reported `ESP32_C61_LEDC_SERVO_OK`, read back `DUTY_R=18992`, and captured two
GPIO6 transitions. The committed result and build/run logs are in
[`evidence/ledc-servo/`](evidence/ledc-servo/).

## Model boundary

The profile covers the HP RISC-V core, 8 MB flash and 320 KiB SRAM windows,
UART0 TX, PCR gates for UART0/I2C0/LEDC, GPIO4/5 I2C0 matrix routes, GPIO6 LEDC
output, GPIO8 direct output, and IO_MUX GPIO4/5/6/8. The LEDC model uses the C61
timer and channel bit positions, routes six waveform lines through matrix
signals 0–5, and models the reset-selected 40 MHz source. The smokes exercise
timer 2/channel 0. The I2C engine performs BME280 register reads, the WS2812B
model decodes the GRB waveform, and the SG90 model observes committed duty.
Other GPIO pads, ROM boot, PSRAM, Wi-Fi, Bluetooth LE, USB Serial/JTAG, and the
LP subsystem remain outside this slice. The evidence is from local
deterministic core runs, not a hosted `model_verified` verdict or physical-board
capture.
