# Espressif ESP32-C3-LCDkit

**Demand rank:** ESP32 is rank #1 in the standing MCU usage evidence (38%).
This round adds the current ESP32-C3-LCDkit board and its 1.28-inch display
daughterboard. It extends the existing C3 model with this board's LCD wiring;
it does not add a new silicon model. Live vendor availability is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The smoke boots C3 firmware, sends GC9A01 commands and pixel data through
SPI2, paints an 8×8 red patch, enables the LCD backlight pin, and prints a
UART0 marker. The descriptor exposes only GPIO0/1/2/5/7, UART0, SPI2, and
IO_MUX. It omits the board's speaker, encoder, IR module, RGB LED, USB, Wi-Fi,
BLE, and other peripherals. The current LCD model does not treat GPIO5 as a
brightness input; the profile drives the board's backlight-control pin high,
while display assertions cover the GC9A01 framebuffer and display-on state.
The exercised path uses real GPIO, SPI2, UART0, and GC9A01 models; none is on
the known-stub allowlist. Omitted board functions are outside this profile.

## Copy-paste Arduino example

In Arduino IDE, install **Adafruit GC9A01A** and its **Adafruit GFX Library**
dependency, select an ESP32-C3 board, and upload:

```cpp
#include <SPI.h>
#include <Adafruit_GFX.h>
#include <Adafruit_GC9A01A.h>

constexpr int LCD_SCK = 1;
constexpr int LCD_MOSI = 0;
constexpr int LCD_DC = 2;
constexpr int LCD_BACKLIGHT = 5;
constexpr int LCD_CS = 7;

Adafruit_GC9A01A display(&SPI, LCD_DC, LCD_CS, -1);

void setup() {
  pinMode(LCD_BACKLIGHT, OUTPUT);
  digitalWrite(LCD_BACKLIGHT, HIGH);
  SPI.begin(LCD_SCK, -1, LCD_MOSI, LCD_CS);
  display.begin();
  display.fillScreen(GC9A01A_BLACK);
  display.fillRect(0, 0, 64, 64, GC9A01A_RED);
}

void loop() {}
```

The pin allocation and GC9A01 model are grounded in Espressif's [live board
guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-lcdkit/user_guide.html).
The Adafruit library supports a caller-supplied SPI peripheral and display
pins; its documentation is in the [GC9A01A library
header](https://github.com/adafruit/Adafruit_GC9A01A/blob/main/Adafruit_GC9A01A.h).

## Run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-lcdkit \
  --crate firmware-esp32c3-lcdkit-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-lcdkit/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-lcdkit.yaml \
  --out-dir examples/esp32c3-lcdkit/evidence
```

The smoke checks the UART marker and a fully inked 8×8 red region on the lit
display. Committed logs and the structured simulator result are under
[`evidence/`](evidence/). This is deterministic model evidence, not a physical
board test. The scope is limited to the LCD path described above. The separate
hosted verifier has no GC9A01 component entry at present; see
[`evidence/hosted-verification.md`](evidence/hosted-verification.md).

The smoke script also writes `onboarding-metrics.json`. To generate a local
scoreboard for this target:

```sh
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/esp32c3-lcdkit/evidence \
  --markdown-out examples/esp32c3-lcdkit/evidence/onboarding-scoreboard.md \
  --json-out examples/esp32c3-lcdkit/evidence/onboarding-scoreboard.json
```
