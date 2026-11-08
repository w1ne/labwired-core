# Arduino Mega 2560 Rev3

This LabWired system profile covers the Arduino Mega 2560 Rev3's ATmega2560
core and a narrow, tested board I/O surface. The [Arduino product page](https://docs.arduino.cc/hardware/mega-2560)
describes the board as a 16 MHz ATmega2560 board with 54 digital I/O, 16 analog
inputs, and four hardware serial ports. The official store listing and live
Microchip lifecycle check are recorded in
[`examples/arduino-mega-2560-io/evidence/vendor-status.md`](../../examples/arduino-mega-2560-io/evidence/vendor-status.md).
The [official Mega 2560 Rev3 pinout](https://docs.arduino.cc/resources/pinouts/A000067-full-pinout.pdf)
maps D13 to PB7 and D0/D1 to PE0/PE1.

## Board mapping and tested behavior

| Board signal | MCU path | Modeled use |
|---|---|---|
| D13 / `LED_BUILTIN` | PB7 (`portb:7`) | Active-high LED output, asserted by the smoke |
| D0 / RX0 | PE0 / USART0 RX | UART0 receive pin; not exercised by this smoke |
| D1 / TX0 | PE1 / USART0 TX | Host-console output from `Serial` |
| A0 | ADC0 | Centered test input, reads 512 |
| A8, A9 | ADC8, ADC9 | Centered 5 V potentiometers in the example system, read 512 |

The root system descriptor is [`configs/systems/arduino-mega-2560.yaml`](../../configs/systems/arduino-mega-2560.yaml).
The runnable example adds the two potentiometers in
[`examples/arduino-mega-2560-io/system.yaml`](../../examples/arduino-mega-2560-io/system.yaml)
and runs `examples/arduino-mega-2560-io/src/main.cpp`. Its smoke checks the
USART0 banner, ADC values, and the PB7 LED edge; see
[`examples/arduino-mega-2560-io/ONBOARDING.md`](../../examples/arduino-mega-2560-io/ONBOARDING.md).

## Scope limits

This profile does not model the board's ATmega16U2 USB-to-serial bridge or USB
enumeration. The host-console connector abstracts USART0 output. The ATmega2560
profile also leaves INTn/PCINT, the Timer3/4/5 PWM pads (D2 D3 D5 D6 D7 D8
D44 D45 D46), OC1C, and USART1/2/3 outside its modeled surface. The Timer0/1/2
pads (D4 D9 D10 D11 D12 D13) carry `analogWrite` PWM. The smoke makes no claim for those features.
