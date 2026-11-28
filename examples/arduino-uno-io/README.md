# Arduino Uno R3 I/O proof

One pin on each ATmega328P port, so a pass means all three reach the canvas:

| Pin | Port | Role |
|-----|------|------|
| D7 | PD7 | output, toggles every loop |
| D2 | PD2 | input with pull-up, button to GND |
| A0 | PC0 / ADC0 | potentiometer wiper |
| D13 | PB5 | `LED_BUILTIN`, lit while the button is held |

The sketch prints `D2=<level> A0=<counts>` every loop over USART0.

```bash
examples/arduino-uno-io/build-firmware.sh
cargo test -p labwired-core --test avr avr_uno_io::
```

The latest board smoke and its metrics are recorded in [`evidence/`](evidence/).

The potentiometer model runs from a 3.3 V rail. Wire its top leg to 3.3V on a
real board as well; full travel then reads about 675 counts on the 5 V reference.

## Analog netlists

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) adds a Beta NTC
divider on A0 (PC0/ADC0) and a Shockley diode load line on A1 (PC1/ADC1), both
solved by the in-core MNA solver and routed as `board.analog.pc0_volts` /
`board.analog.pc1_volts`. The deck names no board, CPU, or channel; the
ATmega328P descriptor's `analog_pins:` map selects the converter input, so the
same file runs on the Arduino Nano and the CONTROLLINO MINI.

```bash
cargo test -p labwired-core --test analog cosim_avr_atmega328p_analog
```

The test solves both circuits at -20/25/85 °C, compares every node against an
independent closed form, and executes the firmware ADMUX/ADCSRA/ADCL/ADCH
sequence so the asserted counts come through the interpreter, not a model
setter.
