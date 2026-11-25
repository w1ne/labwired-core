# Arduino Leonardo mixed-signal deck

The board profile is `configs/systems/arduino-leonardo.yaml` (ATmega32U4 at
16 MHz, 5 V). This directory carries only the analog co-simulation fixture and
the arduino-matrix evidence; it is not a standalone firmware example.

## Mixed-signal inputs

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) wires three
generic netlists to the board:

| Deck | Pad | Arduino header | ADC channel |
|---|---|---|---|
| Beta-parameter NTC divider (10 k, B = 3950) | PF7 | A0 | ADC7 |
| Shockley diode load line (1 k limit) | PF6 | A1 | ADC6 |
| 10 k / 4.7 k divider | PD7 | A7 | ADC10 (needs ADCSRB.MUX5) |

No deck names the board or a channel. The ATmega32U4 descriptor's
`analog_pins:` map selects the channel from the pad label, and the third deck
proves the high half of the map that is only reachable with `ADCSRB.MUX5`.

```sh
cargo test -p labwired-core --test analog cosim_avr_atmega32u4_analog
```

The test solves all three decks at -20/25/85 C, compares each node with an
independent closed form, executes the AVR `analogRead` register sequence, and
asserts the firmware-visible 10-bit code against `round(mV) * 1024 / 5000` at
the board's 5 V AVcc. The analog pad map follows Microchip DS7766 Tables
10-3/10-8/10-13 and 24-4.
