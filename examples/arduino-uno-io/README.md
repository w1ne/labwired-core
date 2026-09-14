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
pio run -d examples/arduino-uno-io
cp examples/arduino-uno-io/.pio/build/uno/firmware.elf tests/fixtures/avr/arduino-uno-io.elf
cargo test -p labwired-core --test avr_uno_io
```

The potentiometer model runs from a 3.3 V rail. Wire its top leg to 3.3V on a
real board as well; full travel then reads about 675 counts on the 5 V reference.
