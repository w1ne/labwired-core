# Arduino Mega 2560 I/O smoke

Arduino-core firmware for the ATmega2560 model (`configs/chips/atmega2560.yaml`).
Every 100 ms it lights one of 12 LEDs (D3 D4 D5 A2 A1 A0 D13 D12 D8 D7 D6 D2,
on ports E, G, F, B and H) and prints one line on Serial:

    k=<led> a8=<analogRead(A8)> a9=<analogRead(A9)> a0=<analogRead(A0)> ms=<millis()> f=<fn>

`system.yaml` puts two 5 V potentiometers on A8 (ADC8) and A9 (ADC9).

    cargo test -p labwired-core --test avr avr_mega2560_io::

The committed ELF (`tests/fixtures/avr/arduino-mega-2560-io.elf`) was built
with arduino-cli, `arduino:avr` 1.8.x, board `arduino:avr:mega`, from
`src/main.cpp` (copied to a `mega_io/mega_io.ino` sketch folder).

`analogWrite` reaches the pads on the six Timer0/1/2 pins: D13 (OC0A), D4
(OC0B), D11/D12 (OC1A/OC1B), D10 (OC2A) and D9 (OC2B), see
`cargo test -p labwired-core --test avr avr_mega2560_pwm::`. Timer1/3/4/5 run
every 16-bit waveform mode with compare A/B/C, overflow and input-capture
interrupts, and drive their PWM pads D5/D2/D3 (OC3A/B/C), D6/D7/D8 (OC4A/B/C),
D46/D45/D44 (OC5A/B/C) and OC1C on D13 through the PB7 modulator, see
`cargo test -p labwired-core --test avr avr_mega2560_timers::`. The stock Arduino matrix
row (`validation/arduino-matrix`, board `atmega2560`) passes L0–L7, evidence in
[`evidence/arduino-matrix/`](evidence/arduino-matrix/).

Not modelled on the Mega: `attachInterrupt` (INTn/PCINT), USART1..3
(`Serial1`..`Serial3`), and on the 16-bit timers external clocking from the
Tn pins, toggle and non-PWM compare-output pads and double-buffered `OCRnx`.
