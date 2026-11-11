# Arduino Mega 2560 I/O smoke

Arduino-core firmware for the ATmega2560 model (`configs/chips/atmega2560.yaml`).
Every 100 ms it lights one of 12 LEDs (D3 D4 D5 A2 A1 A0 D13 D12 D8 D7 D6 D2,
on ports E, G, F, B and H) and prints one line on Serial:

    k=<led> a8=<analogRead(A8)> a9=<analogRead(A9)> a0=<analogRead(A0)> ms=<millis()> f=<fn>

`system.yaml` puts two 5 V potentiometers on A8 (ADC8) and A9 (ADC9).

`analog-ntc-system.yaml` is the separate MNA co-simulation fixture: its
temperature-driven NTC divider routes to ADC8, and
`cosim_pin_routing::arduino_mega_2560_ntc_netlist_reaches_adc8_through_mux5_end_to_end`
checks the solved voltage and the firmware-visible ADC result at three
temperatures.

`analog-bjt-high-injection-system.yaml` routes matched Gummel–Poon NPN
branches with and without IKF/NKF high-injection rolloff to ADC9/ADC10 beside
the unrelated NTC on ADC8. The end-to-end test checks independent collector
load lines and converts all three values through the ATmega2560 MUX5 path:

    cargo test -p labwired-core --test cosim_pin_routing arduino_mega_2560_bjt_high_injection_and_ntc_reach_firmware_adc_through_mux5

    cargo test -p labwired-core --test avr_mega2560_io

The committed ELF (`tests/fixtures/avr/arduino-mega-2560-io.elf`) was built
with arduino-cli, `arduino:avr` 1.8.x, board `arduino:avr:mega`, from
`src/main.cpp` (copied to a `mega_io/mega_io.ino` sketch folder).

Not modelled on the Mega: `attachInterrupt` (INTn/PCINT), `analogWrite` PWM
pads, Timer3/4/5 and USART1..3 (`Serial1`..`Serial3`).
