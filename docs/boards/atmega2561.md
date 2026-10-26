# ATmega2561

64-pin AVR8 part: 256 KB flash, 8 KB SRAM, 54 GPIO, USART0 and eight ADC
channels on LabWired's public AVR interpreter.

## Status at a glance

| Surface | State |
| :--- | :--- |
| CPU (`avr8`, 16 MHz) | ✅ shared public `AvrModel::Atmega2560` core |
| Flash / SRAM | ✅ 256 KB at `0x0`, 8 KB at `0x0200` |
| GPIO | ✅ PA0..PA7, PB0..PB7, PC0..PC7, PD0..PD7, PE0..PE7, PF0..PF7, PG0..PG5 |
| USART | ✅ USART0 console; ❌ USART1 (not modelled) |
| ADC | ✅ ADC0..ADC7; ❌ ADC8..ADC15 (100-pin only) |
| EEPROM | ❌ 4 KB on silicon, no public controller model |
| Timer3/4/5, PWM channels, ports H..L | ❌ 100-pin ATmega2560 features, not present here |

The public CPU alias follows simavr: both route ATmega2561 to the shared
ATmega2560 core. The LabWired descriptor is narrower, so only behaviours the
public interpreter actually implements are exposed.

## Flash / firmware artifact

`tests/fixtures/avr/atmega2561-io.elf` — a tiny 3-byte-PC AVR image (source
sequence in `examples/atmega2561-io/src/main.S`) that prints `2561 OK` on
USART0 and drives PB5 high.

## Pins

The authoritative package map is transcribed in `configs/chips/atmega2561.yaml`
from DS40002211A Figure 1-3: 54 GPIO lines, with PG6/PG7 absent on the 64-pin
package.

## How to run

```bash
cargo test -p labwired-core --test avr_atmega2561
cargo run -q -p labwired-cli -- test \
  --script examples/atmega2561-io/io-smoke.yaml --no-uart-stdout
```

## Sources

- Microchip DS40002211A (ATmega640/V-1280/V-1281/V-2560/V-2561/V):
  §1 Figure 1-3, §2.2 Table 2-1, §2.3, §8, §31.1.
- Public simavr `simavr/cores/sim_mega2560.c` (the public alias).
- `examples/atmega2561-io/REQUIRED_DOCS.md` and `VALIDATION.md`.
