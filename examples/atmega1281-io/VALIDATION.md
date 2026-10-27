# Validation — ATmega1281

## Evidence

| Capability | Evidence | Result |
| :--- | :--- | :--- |
| Exact 128 KB flash, 8 KB SRAM, 54 GPIO and eight ADC inputs | `avr_atmega1281` descriptor assertions | passing |
| Dedicated two-byte PC; EIND/EIJMP/EICALL absent | `avr_atmega1281` CPU assertions and branch tests | passing |
| AVR-GCC reset/startup, USART0 output and PB5 high | `avr_atmega1281_firmware` using `tests/fixtures/avr/atmega1281-io.elf` | `1281 OK` and PB5 high |
| Runnable CLI example | `examples/atmega1281-io/io-smoke.yaml` | 1/1 checks |
| Source reproducibly builds for `-mmcu=atmega1281` | `examples/atmega1281-io/ci/build.sh` | passing with AVR-GCC |
| Silicon capture or physical-board differential | none | not claimed |

## Commands

```bash
examples/atmega1281-io/ci/build.sh
examples/atmega1281-io/ci/test.sh
cargo test -p labwired-core --lib cpu::avr -- --quiet
```

The smoke image is built from `src/main.c` with AVR-GCC and the ATmega1281
startup/vector table. The same checked-in ELF is exercised by the Rust
integration test and CLI script.

## Explicit limits

- EEPROM is present on silicon but has no public controller model.
- USART1 and Timer3 are present on silicon but are not modeled or declared.
- The 64-pin profile exposes 54 GPIO pads on ports A..G and ADC0..ADC7 only.
- PWM behavior is not modeled. Timer4/5, ports H..L, and ADC8..ADC15 belong to
  the 100-pin sibling and are outside this part profile.
- No silicon capture, timing differential, or physical carrier validation is
  included.
