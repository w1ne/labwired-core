# Required Documents — ATmega1281 on STK600

| Source | Used for |
| :--- | :--- |
| [Microchip STK600 AVR Flash MCU Starter Kit User's Guide, DS40001904H](https://ww1.microchip.com/downloads/aemDocuments/documents/MCU08/ProductDocuments/UserGuides/STK600-AVR-Flash-MCU-Starter-Kit-Users-Guide-DS40001904.pdf) | Device Support table: ATmega1281 with RC09 + SC02; §6.6: LED and switch headers, active-low behavior, and jumper routing. |
| [Microchip ATmega640/1280/1281/2560/2561 datasheet, DS40002211A](https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ProductDocuments/DataSheets/ATmega640-1280-1281-2560-2561-Datasheet-DS40002211A.pdf) | ATmega1281 package pins, USART0/GPIO registers, clock limit, flash and SRAM geometry. |
| [Microchip STK600-SC02 product page](https://www.microchip.com/en-us/development-tool/atstk600-sc02) | SC02 is the generic TQFP64 socket card. |
| configs/chips/atmega1281.yaml | Reused chip descriptor; unchanged by this board example. |
| configs/systems/stk600-atmega1281.yaml | Canonical board profile and demo jumper harness. |
| src/main.c | Demo firmware source for active-low SW0/LED0 and USART0 output. |
| crates/core/src/cpu/avr.rs and crates/core/src/peripherals/avr_gpio.rs | Simulator CPU and GPIO behavior exercised by the integration test. |

The Microchip guide describes the kit's headers and polarity. PB5 and PB0 are chosen as
explicit example jumpers; they are not implied to be factory-connected to LED0/SW0.
