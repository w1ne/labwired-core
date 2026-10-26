# Validation — ATmega2561

Claims, evidence, and what is explicitly **not** claimed.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| 64-pin descriptor loads with the exact part geometry | smoke-tested | `avr_atmega2561::descriptor_is_the_64_pin_2561_and_not_the_100_pin_2560` |
| 256 KB flash, 8 KB SRAM at `0x0200`, 3-byte PC | smoke-tested | `avr_atmega2561::chip_name_selects_the_public_mega_core_with_2561_geometry` |
| Boot, USART0 output, PB5 output | smoke-tested | committed `tests/fixtures/avr/atmega2561-io.elf`, `avr_atmega2561::atmega2561_boots_and_prints` |
| 54-pin A..G map | smoke-tested | 54 entries in `configs/chips/atmega2561.yaml`, checked by the descriptor test |
| ADC0..ADC7 only | smoke-tested | `avr_atmega2561::only_eight_adc_inputs_are_mapped` |
| CLI script | smoke-tested | `examples/atmega2561-io/io-smoke.yaml` via `examples/atmega2561-io/ci/test.sh` |
| Silicon capture / executing-fidelity differential | **none** | SIM-DERIVED: no bench ATmega2561 was captured |

## Commands

```bash
# from core/
cargo test -p labwired-core --test avr_atmega2561
cargo run -q -p labwired-cli -- test \
  --script examples/atmega2561-io/io-smoke.yaml --no-uart-stdout
```

Expected: all tests pass; the CLI script reports `1/1 checks` and the fixture
prints `2561 OK`.

## Explicit non-claims

- **EEPROM**: DS40002211A gives the part 4 KB, but the public interpreter has
  no EEPROM controller and the descriptor declares no EEPROM window. Firmware
  that reads or writes `EECR`/`EEDR`/`EEAR` is not supported.
- **USART1**: the silicon has two USARTs; only USART0 is modelled and
  declared. `Serial1`-style firmware is out of scope.
- **Ports H..L, ADC8..ADC15, Timer3/4/5, 12 PWM channels**: these are 100-pin
  ATmega2560 features. They are not present on the 64-pin part and are not
  exposed.
- **PG6/PG7**: Port G is six bits on this package. The register model is
  byte-wide because it is shared AVR hardware, but the authoritative pin map
  declares only PG0..PG5.
