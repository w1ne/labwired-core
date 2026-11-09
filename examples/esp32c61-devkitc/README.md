# ESP32-C61-DevKitC-1 v2.0 (N8R2) + BME280

**Demand rank:** ESP32 family, rank 1 (38% of respondents use an ESP32). This
round adds board-peripheral depth to the already supported C61 DevKitC-1; it is
not a new chip model. The measured family demand makes a routed I2C sensor path
on this active board more valuable than another low-demand variant.

The board smoke runs bare-metal firmware built for `riscv32imc` (an instruction
subset of the C61's RV32IMAC core). It reads the BME280 ID and raw temperature
over I2C0 on header GPIO4/GPIO5, prints the UART0 and sensor markers, and
bit-bangs a red WS2812B frame to the board's GPIO8 RGB LED. The board guide
connects UART0 to its USB-to-UART bridge, identifies GPIO4/5 on header J1, and
the official schematic identifies the GPIO8 LED as D6 WS2812B.

## BME280 wiring

Connect an Adafruit BME280 breakout to 3V3 and GND, SDA to GPIO4 (J1 pin 3),
and SCL to GPIO5 (J1 pin 4). The configured address is `0x77`; set the
breakout's address jumper accordingly. The simulated system also keeps the
onboard RGB LED on GPIO8.

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

The 2026-11-09 local run passed **6/6 assertions** at 1,000,000 steps. UART
reported `BME280_ID=0x60` and `BME280_TEMP_RAW=0x082DC0`; the GPIO8 RGB
waveform decoded as red.

## Model boundary

The profile covers the HP RISC-V core, 8 MB flash and 320 KiB SRAM windows,
UART0's register path and TX FIFO, the UART0/I2C0 PCR clock bits, GPIO4/5 I2C0
matrix routes, GPIO8 output, and the GPIO4/5/8 IO_MUX function selects. The
existing I2C engine performs the BME280 register reads, and the WS2812B model
decodes the actual GRB waveform. Other GPIO pads, ROM boot, PSRAM, Wi-Fi,
Bluetooth LE, USB Serial/JTAG, and the LP subsystem remain outside this slice.
The smoke log is a local deterministic core run; there is no hosted
`model_verified` verdict or physical-board confirmation.
