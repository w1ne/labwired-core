# ESP32-C3-DevKitM-1-N4X UART and RGB smoke

ESP32 family demand rank: **#1 (38% of MCU usage in the 2025 survey)**; C3-
specific use is not separately measured. This is a current, buyable official
Espressif board variant. The live guide marks the unsuffixed DevKitM-1 SKU EOL,
so this profile is specifically for the **ESP32-C3-DevKitM-1-N4X**.

The profile fast-loads a bare RISC-V ELF into the 4 MiB flash window, writes a
marker through UART0, and sends a red WS2812 frame through GPIO8. The attached
NeoPixel model must decode one lit red pixel. Espressif's board guide maps the
onboard addressable RGB LED to GPIO8.

UART0 capture goes directly to the simulator host console. The smoke does not
claim the physical USB-to-UART bridge, the C3 mask-ROM/bootloader path, or
IO_MUX electrical routing. The RGB assertion proves GPIO output edges and
WS2812 protocol decoding; it does not model the LED's power draw, optical
output, or timed RMT playback. The C3's minimal RMT register model is not used
by this bit-banged smoke.

## Pin map

| Board signal | ESP32-C3 pad | Profile behavior |
|---|---:|---|
| Onboard addressable RGB LED data | GPIO8 | Attached WS2812 observer decodes the bit stream |
| UART0 TXD | GPIO21 | UART0 output is captured by the simulator host console |
| UART0 RXD | GPIO20 | Pin identity documented; not exercised by this smoke |

Espressif's [live board guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkitm-1/index.html)
contains the full header pin layout. The profile captures UART0 at the SoC
peripheral; it does not simulate the board's USB-to-UART bridge or its pad
electrical route.

## Build and run

```sh
rustup target add riscv32imc-unknown-none-elf
examples/esp32c3-devkitm-1-n4x/ci/test.sh
```

The smoke expects UART marker `C3_N4X_RGB_OK`, observes GPIO8 transitions, and
requires the WS2812 observer to decode a single red pixel. The committed output
is under [`evidence/`](evidence/).

## Copy-paste Arduino example

Install the **Adafruit NeoPixel** library, select **ESP32-C3-DevKitM-1** in the
Arduino ESP32 core, then upload this sketch:

```cpp
#include <Adafruit_NeoPixel.h>
#include <Arduino.h>

constexpr uint8_t RGB_PIN = 8;
Adafruit_NeoPixel rgb(1, RGB_PIN, NEO_GRB + NEO_KHZ800);

void setup() {
  Serial.begin(115200);
  rgb.begin();
  rgb.setPixelColor(0, rgb.Color(255, 0, 0));
  rgb.show();
  Serial.println("C3_N4X_RGB_OK");
}

void loop() {}
```

## Analog inputs

`configs/chips/esp32c3.yaml` publishes the SAR ADC1 pad map: GPIO0..GPIO4 are
ADC1_CH0..CH4 on `apb_saradc`. GPIO5 (ADC2_CH0) is not on the map, because
ESP-IDF refuses ADC2 one-shot reads on the C3. A co-simulation deck names a
pad only, as `board.analog.<pad>_volts`, and the router finds the channel.

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) is the generic
deck: a Beta NTC divider on GPIO3 and a Shockley diode load line on GPIO4.
`crates/core/tests/analog/cosim_esp32c3_boards_analog.rs` runs it on all
eleven ESP32-C3 profiles. It asserts the solved voltages and the
`SAR1DATA_STATUS` codes of the one-shot conversion (25 C NTC = 2047 counts):

```bash
cargo test -p labwired-core --test analog esp32c3
```

The ADC model is behavioural: 12 bits on a fixed 3.3 V reference. It does not
model attenuation, eFuse calibration or the SAR input impedance.

## Scope

- Modeled in this profile: C3 fast-boot ELF loading, UART0 FIFO/console, GPIO
  output register transitions, and a one-pixel WS2812 observer on GPIO8.
- Not modeled in this profile: mask-ROM/bootloader startup, USB-UART bridge,
  IO_MUX pad electrical routing, reset/boot buttons, Wi-Fi, Bluetooth LE, and
  power behavior. The C3 RMT model is minimal and its timed waveform path is
  not exercised.
- This is simulator evidence, not a live-silicon differential.

Vendor availability evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).
