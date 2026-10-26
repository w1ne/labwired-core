# Required Documents — SAM4S Xplained

Authoritative sources used by this example (retrieved 2026-10-26):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| SAM4S datasheet (Atmel 11100) | SAM4S series | §31 PIO (PER/PDR/PSR, OER/ODR/OSR, SODR/CODR/ODSR, PDSR, PUER/PUDR/PUSR), §28 PMC (PCER0/PCSR0, MCKR, SR MOSCXTS/MCKRDY), §34/§35 UART/USART (CR/MR/SR/THR/BRGR), memory map, 1 MiB flash / 128 KiB SRAM, 120 MHz |
| [Zephyr SAM4S Xplained board documentation](https://docs.zephyrproject.org/latest/boards/atmel/sam/sam4s_xplained/doc/index.html) | board wiring and console | UART0 on PA9/PA10 is connected to the onboard SEGGER J-Link OB and exposed as a virtual COM port |
| Renode `platforms/cpus/sam4s.repl` + `sam4s16c.repl` | the SoC definition | peripheral bases (PMC 0x400E0400, UART0 0x400E0600, PIOA/B/C 0x400E0E00/0x400E1000/0x400E1200, USART0/1 0x40024000/0x40028000) and the part sizes (1 MiB/128 KiB) |
| Renode `platforms/boards/sam4s_xplained.repl` + `tests/platforms/SAM4S_XPLAINED.robot` | the board | LED on PIOC pin 10, button on PIOA pin 5, console `sysbus.uart0` |

Open items: the TCs, SPI0, TWI0/TWI1, EEFC, the RSTC/SUPC/RTT/WDT/RTC system
controller and CHIPID are declared stubs; the PIO's peripheral mux (ABSR),
multi-drive/open-drain, debounce and the programmable clock are stored but not
modelled.
