# Arduino Mega 2560 I/O smoke

Arduino-core firmware for the ATmega2560 model (`configs/chips/atmega2560.yaml`).
Every 100 ms it lights one of 12 LEDs (D3 D4 D5 A2 A1 A0 D13 D12 D8 D7 D6 D2,
on ports E, G, F, B and H) and prints one line on Serial:

    k=<led> a8=<analogRead(A8)> a9=<analogRead(A9)> a0=<analogRead(A0)> ms=<millis()> f=<fn>

`system.yaml` puts two 5 V potentiometers on A8 (ADC8) and A9 (ADC9).
The firmware also lights the built-in D13 LED during setup; the smoke asserts
the PB7 edge as well as the UART and ADC report.

    cargo test -p labwired-core --test avr_mega2560_io

For the reproducible board onboarding build and committed evidence, see
[`ONBOARDING.md`](ONBOARDING.md).

The committed ELF (`tests/fixtures/avr/arduino-mega-2560-io.elf`) is rebuilt
from `src/main.cpp` with PlatformIO (`atmelavr@5.3.0`, `megaatmega2560`) by
[`ci/build-firmware.sh`](ci/build-firmware.sh).

`analogWrite` reaches the pads on the six Timer0/1/2 pins: D13 (OC0A), D4
(OC0B), D11/D12 (OC1A/OC1B), D10 (OC2A) and D9 (OC2B), see
`cargo test -p labwired-core --test avr_mega2560_pwm`. The stock Arduino matrix
row (`validation/arduino-matrix`, board `atmega2560`) passes L0–L7, evidence in
[`evidence/arduino-matrix/`](evidence/arduino-matrix/).

Not modelled on the Mega: `attachInterrupt` (INTn/PCINT), PWM on the
Timer3/4/5 pins (D2 D3 D5 D6 D7 D8 D44 D45 D46: those pads follow `PORTx`),
OC1C, and USART1..3 (`Serial1`..`Serial3`).
