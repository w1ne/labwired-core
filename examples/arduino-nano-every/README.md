# Arduino Nano Every / ATmega4809

Demand position: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. This board uses ATmega4809, which was not separately ranked; the
survey's #2 MCU response is ATmega328, not ATmega4809.

This quickstart uses the stock Arduino API on the real Nano Every target. It
prints `NANO_EVERY_OK` through the board's USB bridge and toggles
`LED_BUILTIN` (D13 / PE2). The exact sketch below is built and run by the
committed smoke. The 1 ms delay keeps the smoke short; change both delays to
`250` for a human-visible blink.

```cpp
#include <Arduino.h>

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  Serial.println("NANO_EVERY_OK");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1);
}
```

The chip descriptor (`configs/chips/atmega4809.yaml`) now models more than
this example uses. Stock ArduinoCore-megaavr sketches run on it in the Arduino
matrix (`validation/arduino-matrix`, row `arduino-nano-every`): L0/L1 serial
through USART3, L2 blink and L7 `millis()`/`micros()` through TCB3 pass. L3
`Wire` reads an INA219's registers over TWI0 (host mode), L4 `SPI` reads a
MAX31855 frame over SPI0 (host mode), and L5 `analogRead(A0)` converts on
ADC0; their evidence and negative controls are under
`evidence/arduino-matrix/`. The timer pad outputs are not modelled yet, so L6
is skipped with that reason (L8: the part has no CAN). An I/O access to anything the descriptor does not declare faults
instead of reading zero.

## Lifecycle and hardware sources

- Microchip's live product check for ATMEGA4809 reported **In Production** on
  2026-11-08. The vendor page is
  [ATmega4809](https://www.microchip.com/en-us/product/ATMEGA4809).
- Arduino's [Nano Every store page](https://store.arduino.cc/products/nano-every)
  listed product ABX00028 at €14.50 with an Add to cart option when checked on
  2026-11-08.
- Arduino's official [board package](https://github.com/arduino/ArduinoCore-megaavr/blob/master/boards.txt)
  selects ATmega4809 at 16 MHz with 48 KB flash and 6 KB RAM.
- The official [Nano Every pin map](https://github.com/arduino/ArduinoCore-megaavr/blob/master/variants/nona4809/pins_arduino.h)
  maps `LED_BUILTIN` D13 to PE2.
- Microchip's [ATmega4808/4809 data sheet](https://ww1.microchip.com/downloads/en/DeviceDoc/ATmega4808-4809-Data-Sheet-DS40002173A.pdf)
  gives the ATmega4809 SRAM data-space start at 0x2800 and size 6 KB.

The recorded vendor lifecycle command output is in `evidence/vendor-status.md`.

## Build and smoke

Build the copyable Arduino example with PlatformIO, then run the repo's
`io-smoke.yaml`. It requires the USART3 marker and eight observed PE2 edges:

```sh
./firmware/build.sh
cargo run -q -p labwired-cli -- test --script io-smoke.yaml --no-uart-stdout
```

The committed smoke log and structured result are under `evidence/`. The
Arduino matrix also passes L0/L1/L2/L7 on the stock ArduinoCore-megaavr path.
This is repository smoke evidence, not a `labwired_verify` model-verification
verdict; the hosted verifier catalog has no ATmega4809 target.

The run records `unpowered_rail_assumed`: there is no routed Nano Every power
net in this narrow system, so VDD is idealized at 5 V and power-on/brown-out
reset behavior is not exercised. The hosted verifier catalog also has no
ATmega4809 target, so this round has local CLI evidence only.
