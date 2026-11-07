# ESP32-C61-DevKitC-1 v2.0 (N8R2)

**Demand rank:** ESP32 family, rank 1 (38% of respondents use an ESP32). This
round adds board-peripheral depth to the already supported C61 DevKitC-1; it is
not a new chip model. The measured family demand makes its documented onboard
RGB LED a higher-value extension than another low-demand variant.

The board smoke runs bare-metal firmware built for `riscv32imc` (an instruction
subset of the C61's RV32IMAC core). It checks the UART0 marker and bit-bangs a
red WS2812B frame to the board's GPIO8 RGB LED. The board guide connects UART0
to its USB-to-UART bridge and the official schematic identifies the GPIO8 LED
as D6 WS2812B.

## Arduino sketch example

The official [Arduino-ESP32 core supports C61](https://github.com/espressif/arduino-esp32/blob/master/docs/en/getting_started.rst), but using it currently requires an ESP-IDF component build or rebuilt core libraries; it is not a stock Arduino IDE board package. With the Adafruit NeoPixel library installed, this sketch prints through UART0 and sets the onboard pixel red:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel rgb(1, 8, NEO_GRB + NEO_KHZ800);

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_C61_UART0_OK");
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

## Model boundary

The profile covers the HP RISC-V core, 8 MB flash and 320 KiB SRAM windows,
UART0's register path and TX FIFO, the PCR UART0 clock bits, and the direct
GPIO8 output path (GPIO OUT latch, output-enable W1TS, GPIO8 output selector
and OE_SEL, and IO_MUX GPIO8 function select). The WS2812B model decodes the actual GRB
waveform and the smoke asserts a red pixel. GPIO matrix routing, other GPIO
pads, ROM boot, PSRAM, Wi-Fi, Bluetooth LE, USB Serial/JTAG, and the LP
subsystem are outside this slice. The evidence is a local deterministic core
smoke; there is no hosted `model_verified` verdict or physical-board
confirmation.
