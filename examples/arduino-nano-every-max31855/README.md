# Arduino Nano Every + MAX31855

Run commands from the repository root.

Demand position: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. The ATmega4809 is not ranked separately. The survey's #2 MCU is the
ATmega328, which Microchip marks Not Recommended for New Designs; the Nano
Every is the in-production Arduino board in the same form factor. Live stock
evidence is in [`vendor-status.md`](vendor-status.md).

This example adds a real converter on `SPI`. The stock Adafruit MAX31855
library frames two 32-bit reads with its chip-select pin, decodes the
thermocouple and cold-junction fields, and prints both temperatures.

## Hardware and wiring

Use an Arduino Nano Every (ABX00028) and the Adafruit MAX31855 breakout
(PID 269) with a K-type thermocouple. The breakout has a regulator and level
shifter, so it is safe on the 5 V board.

| Nano Every | MAX31855 breakout | Purpose |
| --- | --- | --- |
| 5V | Vin | breakout supply |
| GND | GND | common ground |
| D13 (PE2) | CLK | SPI0 SCK |
| D12 (PE1) | DO | SPI0 MISO |
| D10 (PB1) | CS | chip select |

D13 is also `LED_BUILTIN`. The sketch does not use the LED, because the pin
carries the SPI clock.

## Copy-paste Arduino example

Select **Arduino Nano Every**, install **Adafruit MAX31855 library** (1.4.2
was used here, with Adafruit BusIO 1.17.4), and upload:

```cpp
#include <Arduino.h>
#include <SPI.h>
#include <Adafruit_MAX31855.h>

static const uint8_t kChipSelect = 10;

Adafruit_MAX31855 thermocouple(kChipSelect);

void setup() {
  Serial.begin(115200);
  thermocouple.begin();
  Serial.println("NANO_EVERY_MAX31855");
  double hot = thermocouple.readCelsius();
  if (isnan(hot)) {
    Serial.print("MAX31855 fault 0x");
    Serial.println(thermocouple.readError(), HEX);
    return;
  }
  Serial.print("TC=");
  Serial.print(hot, 2);
  Serial.println(" C");
  Serial.print("CJ=");
  Serial.print(thermocouple.readInternal(), 4);
  Serial.println(" C");
}

void loop() {}
```

The exact sketch is [`firmware/src/main.cpp`](firmware/src/main.cpp). The
committed build is `tests/fixtures/avr/arduino-nano-every-max31855.elf`;
`firmware/build.sh` rebuilds it and copies it there.

## Build and simulate

```bash
python -m pip install platformio
examples/arduino-nano-every-max31855/firmware/build.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-every-max31855/max31855-smoke.yaml --no-uart-stdout
cargo test -p labwired-core --test avr avr_nano_every_max31855::
```

Expected serial output with the converter model at its defaults (hot junction
25 °C, cold junction 22 °C):

```text
NANO_EVERY_MAX31855
TC=25.00 C
CJ=22.0000 C
```

The Rust test runs on pull requests from the committed ELF. It needs no
PlatformIO.

## Evidence

| Run | Result | Record |
| --- | --- | --- |
| `max31855-smoke.yaml` | PASS, 5/5 checks | [`evidence/run/`](evidence/run/) |
| Value control, hot junction 100.25 °C, cold junction 30.5 °C ([`controls/hot-junction.yaml`](controls/hot-junction.yaml)) | PASS, prints `TC=100.25 C`, `CJ=30.5000 C` | [`evidence/controls/hot-junction/`](evidence/controls/hot-junction/) |
| Control, converter CS on D9 (PB0) ([`controls/wrong-chip-select.yaml`](controls/wrong-chip-select.yaml)) | FAIL as required, prints `MAX31855 fault 0x7` | [`evidence/controls/wrong-chip-select/`](evidence/controls/wrong-chip-select/) |
| Control, no converter on SPI0 ([`controls/no-converter.yaml`](controls/no-converter.yaml)) | FAIL as required, prints `MAX31855 fault 0x7` | [`evidence/controls/no-converter/`](evidence/controls/no-converter/) |
| Control, descriptor before this change (no SPI0 window) | FAIL as required, `Memory access violation at 0x8c1` (SPI0.CTRLB) at `SPI.begin()` | [`evidence/controls/no-spi0-model/`](evidence/controls/no-spi0-model/) |
| Control, chip-select edge at the port write disabled | 2 of 4 Rust tests FAIL as required, second frame prints `CJ=-0.0625 C` | [`evidence/controls/no-chip-select-edges/`](evidence/controls/no-chip-select-edges/) |

This is repository smoke evidence. It is not a `labwired_verify`
model-verification verdict: the hosted verifier catalog has no ATmega4809
target. No physical board test was run.

## Model boundary

Modelled and exercised: the SPI0 host in Normal mode (`megaavr_spi`): one
8-bit transfer per DATA write at the PRESC / CLK2X bit time, the IF flag and
its read-then-access clear sequence that ArduinoCore-megaavr's
`SPI.transfer()` uses, the `PORTMUX.TWISPIROUTEA` route to PE[3:0], the SCK
pad direction, and chip select from the PB1 output level; USART3 for
`Serial`; the MAX31855 32-bit frame.

Modelled and unit-tested only: DORD (LSB first), WRCOL, the SPI0 interrupt,
the MOSI pad direction, the PA[7:4] and PC[3:0] routes.

Refused with an error, not ignored: SPI0 Buffer mode and client mode.

Not modelled: the clock phase and polarity (`SPI_MODE0`..`3` are stored;
the converter answers in every mode), SCK / MOSI / MISO levels on the pads,
the multi-host SS input, MAX31855 conversion time and its fault bits (the
model is always fault-free), and thermocouple linearity. With interrupts
enabled the model clears IF when it raises the vector, not when the vector
executes. The TCA0 / TCB waveform outputs are still not modelled on this
chip. The run assumes an ideal 5 V rail.
