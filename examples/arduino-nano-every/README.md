# Arduino Nano Every / ATmega4809

Demand rank: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. ATmega4809 does not have a separate chip rank in that survey.

This example is the register-level LED smoke: it toggles `LED_BUILTIN` (PE2)
through VPORT E, which the AVRxt core aliases onto the PORTE block.

The chip descriptor (`configs/chips/atmega4809.yaml`) now models more than
this example uses. Stock ArduinoCore-megaavr sketches run on it in the Arduino
matrix (`validation/arduino-matrix`, row `arduino-nano-every`): L0/L1 serial
through USART3, L2 blink and L7 `millis()`/`micros()` through TCB3 pass. TWI0,
SPI0, ADC conversion and timer pad outputs are not modelled yet, so L3-L6 are
skipped with that reason. An I/O access to anything the descriptor does not
declare faults instead of reading zero.

## Lifecycle and hardware sources

- Microchip's live product check for ATMEGA4809 reported **In Production** on
  2026-11-06. The vendor page is
  [ATmega4809](https://www.microchip.com/en-us/product/ATMEGA4809).
- Arduino's [Nano Every store page](https://store.arduino.cc/collections/maker-solutions/products/nano-every)
  listed product ABX00028 at €14.50 with an active Add to cart option when
  checked on 2026-11-06.
- Arduino's official [board package](https://github.com/arduino/ArduinoCore-megaavr/blob/master/boards.txt)
  selects ATmega4809 at 16 MHz with 48 KB flash and 6 KB RAM.
- The official [Nano Every pin map](https://github.com/arduino/ArduinoCore-megaavr/blob/master/variants/nona4809/pins_arduino.h)
  maps `LED_BUILTIN` D13 to PE2.
- Microchip's [ATmega4808/4809 data sheet](https://ww1.microchip.com/downloads/en/DeviceDoc/ATmega4808-4809-Data-Sheet-DS40002173A.pdf)
  gives the ATmega4809 SRAM data-space start at 0x2800 and size 6 KB.

The recorded vendor lifecycle command output is in `evidence/vendor-status.md`.

## Build and smoke

Build the copyable register-level example with PlatformIO, then run the repo's
`io-smoke.yaml`. It toggles PE2 and requires eight observed edges:

```sh
./firmware/build.sh
cargo run -q -p labwired-cli -- test --script io-smoke.yaml --no-uart-stdout
```

The committed smoke log and structured result are under `evidence/`. This is a
repository smoke result, not a LabWired model-verification verdict.

The run records `unpowered_rail_assumed`: there is no routed Nano Every power
net in this narrow system, so VDD is idealized at 5 V and power-on/brown-out
reset behavior is not exercised. The hosted verifier catalog also has no
ATmega4809 target, so this round has local CLI evidence only.
