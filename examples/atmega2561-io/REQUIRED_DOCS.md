# Required Documents — ATmega2561

Authoritative sources used by this descriptor and example:

| Doc | Title | Used for |
| :--- | :--- | :--- |
| Microchip DS40002211A | ATmega640/V-1280/V-1281/V-2560/V-2561/V datasheet | §1 Figure 1-3 (64-pin TQFP/QFN pinout), §2.2 Table 2-1 (256 KB flash, 4 KB EEPROM, 8 KB SRAM, 54 GPIO, 2 USARTs, 8 ADC channels), §2.3 (ports), §8 (memory map), §31.1 (VIL/VIH) |
| [public simavr `sim_mega2560.c`](https://github.com/buserror/simavr/blob/5b08da60497f7564f50941f74a549fe2851db030/simavr/cores/sim_mega2560.c) | public open-source model | public alias of ATmega2561 to the shared ATmega2560 core; LabWired routes the chip name the same way |
| [`crates/core/src/cpu/avr.rs`](../../crates/core/src/cpu/avr.rs) | LabWired public AVR interpreter | `AvrModel::Atmega2560`: 256 KB flash, 3-byte PC, SRAM at `0x0200`, USART0/ADC behaviour actually implemented |

The datasheet is the source of the part facts. simavr is the source of the
public CPU alias; no simavr code was copied.
