# ATmega1281

The 64-pin ATmega1281 profile has 128 KB flash, 8 KB SRAM, 54 GPIO lines,
USART0, and eight bonded ADC inputs. It uses `AvrModel::Atmega1281`, with a
16-bit word PC and two-byte return addresses.

| Surface | State |
| --- | --- |
| CPU and 16 MHz clock | AVR-GCC reset image boots on the dedicated model |
| Flash / SRAM | 128 KB at 0; 8 KB at `0x0200` |
| Return PC | 16-bit word PC; CALL/RET use two stack bytes |
| GPIO | PA0..PA7, PB0..PB7, PC0..PC7, PD0..PD7, PE0..PE7, PF0..PF7, PG0..PG5 |
| USART | USART0 TX is exercised; USART1 is not modeled |
| ADC | ADC0..ADC7 are declared; the 64-pin profile has no ADC8..ADC15 |
| EIND instructions | EIJMP/EICALL are absent on ATmega1281 |
| EEPROM / Timer3 / PWM | Present in silicon; behavior is not modeled |
| Ports H..L and Timer4/5 | 100-pin sibling features, absent from this profile |

The descriptor exposes only behavior implemented by the public simulator. The
54 GPIO pads follow DS40002211A Figure 1-3. PG6/PG7 and all 100-pin ports are
omitted. EEPROM has no controller, USART1 has no driver or console path, and
Timer3/PWM behavior is not modeled.

## Verification

`examples/atmega1281-io/src/main.c` builds with `avr-gcc -mmcu=atmega1281`.
The committed ELF boots from its reset vector, prints `1281 OK` on USART0, and
drives PB5 high. Rust integration coverage checks that output against the exact
system descriptor, and `examples/atmega1281-io/io-smoke.yaml` checks the same
image through the CLI. No silicon capture or physical-board acceptance is
claimed.

Run `examples/atmega1281-io/ci/build.sh` and
`examples/atmega1281-io/ci/test.sh` from the core repository.

## Sources

- [Microchip ATmega640/1280/1281/2560/2561 datasheet DS40002211A](https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ProductDocuments/DataSheets/ATmega640-1280-1281-2560-2561-Datasheet-DS40002211A.pdf): package geometry, memory, PC width, EIND instructions and peripheral set.
- [Microchip ATmega1281 product page](https://www.microchip.com/en-us/product/ATmega1281).
