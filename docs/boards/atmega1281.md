# ATmega1281

The 64-pin ATmega1281 has 128 KB flash, 8 KB SRAM, 54 GPIO lines, USART0,
and eight bonded ADC inputs. This public profile uses the shared AVR mega
peripheral implementation with exact 1281 memory and package geometry.

| Surface | State |
| --- | --- |
| CPU (avr8, 16 MHz) | AvrModel::Atmega1281 |
| Flash / SRAM | 128 KB at address 0; 8 KB at 0x0200 |
| Return PC | 16-bit word PC; CALL/RET push and pop two bytes |
| GPIO | PA0..PA7, PB0..PB7, PC0..PC7, PD0..PD7, PE0..PE7, PF0..PF7, PG0..PG5 |
| USART | USART0; USART1 is not modelled |
| ADC | ADC0..ADC7; ADC8..ADC15 are not bonded on the 64-pin part |
| EIND instructions | EIJMP/EICALL are absent on ATmega1281 |
| EEPROM | Present on silicon (4 KB); no public controller model |
| Timer3 | Present on silicon; not modelled |
| Ports H..L, ADC8..ADC15, Timer4/5 | 100-pin sibling features |
| PWM | Datasheet lists six 16-bit-resolution channels; PWM behavior is not modelled |

The descriptor uses the 54 package GPIO lines from DS40002211A Figure 1-3.
PG6/PG7 and all 100-pin ports are omitted. The CPU core shares mega-family
register behavior, but the descriptor exposes only peripherals represented by
the public simulator.

## Verification

Run cargo test -p labwired-core --lib cpu::avr and
cargo test -p labwired-core --test avr_atmega1281.

The integration smoke executes a short hand-encoded AVR program through the
1281 descriptor and confirms PB5 reaches the status LED. No avr-gcc compiler
or compiled ATmega1281 firmware fixture was available on work-1, so this is
instruction-level simulator evidence, not a compiled firmware smoke or silicon
capture.

## Sources

- Microchip [ATmega640/1280/1281/2560/2561 datasheet DS40002211A](https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ProductDocuments/DataSheets/ATmega640-1280-1281-2560-2561-Datasheet-DS40002211A.pdf), sections 1, 2.2, 7.6, 7.8, 8.1, 29.6.15, 34:
  64-pin geometry, memory sizes, PC width, EIND instruction availability,
  and peripheral set.
- https://www.microchip.com/en-us/product/ATmega1281
