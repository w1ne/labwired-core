# ATmega2561 I/O smoke

The 64-pin ATmega2561 on LabWired's public AVR interpreter
(`configs/chips/atmega2561.yaml`). The chip name selects the shared
`AvrModel::Atmega2560` core, which provides the 3-byte PC, 256 KB flash and
8 KB SRAM at `0x0200`; the descriptor narrows the public surface to this SKU's
54 GPIO lines, USART0 and ADC0..ADC7.

The committed image (`tests/fixtures/avr/atmega2561-io.elf`, sequence in
`src/main.S`) sets SP to `0x21FF`, prints one line on USART0 and drives PB5
high:

```
2561 OK
```

```bash
cargo test -p labwired-core --test avr_atmega2561
cargo run -q -p labwired-cli -- test \
  --script examples/atmega2561-io/io-smoke.yaml --no-uart-stdout
```

## Analog input example

[`analog-diode-system.yaml`](analog-diode-system.yaml) routes a parameterized
Shockley diode load line into the standalone part's explicit ADC0 input. The
end-to-end test checks the solved junction voltage against an independent
load-line calculation, then checks the value returned by the AVR ADC registers:

```bash
cargo test -p labwired-core --test cosim_pin_routing \
  atmega2561_diode_netlist_reaches_adc0_end_to_end -- --nocapture
```

[`analog-bjt-high-injection-system.yaml`](analog-bjt-high-injection-system.yaml)
routes matched generic Gummel–Poon NPN branches to ADC2/3 beside an unrelated
Beta NTC on ADC0 and Shockley diode on ADC1. The test compares each collector
voltage with an independent 5 V load-line calculation and converts all four
inputs through the ATmega2561 ADC registers:

```bash
cargo test -p labwired-core --test cosim_pin_routing \
  atmega2561_bjt_high_injection_reaches_firmware_adc_alongside_ntc_and_diode -- --nocapture
```

## What this proves

| Surface | State |
| :--- | :--- |
| ATmega2561 / 64-pin geometry (256 KB flash, 8 KB SRAM at `0x0200`) | modelled by the shared public AVR core |
| Ports A..F (8 bits) and PG0..PG5 (54 GPIO) | `avr_gpio` mirrors, exact pin map |
| USART0 console | modelled on the CPU; bytes reach the capture sink |
| ADC0..ADC7 inputs | `avr_adc` with eight channels |
| 4 KB EEPROM | **not modelled** — no EECR/EEDR/EEAR controller is exposed |
| USART1 | **not declared** — registers exist only as untested storage on the shared core |
| Timer3 | **not modelled** — present on the 2561 silicon, but the shared core provides no timer behaviour |
| Ports H..L, ADC8..ADC15, Timer4/5 | 100-pin ATmega2560 features, not present on the 64-pin part |
| PWM | DS40002211A Table 2-1 lists six 16-bit-resolution channels; no PWM behavior is modelled |

Sources and explicit non-claims: `REQUIRED_DOCS.md`, `VALIDATION.md`.
