# ATmega328PB Xplained Mini live availability

Checked 2026-11-06 UTC.

- Microchip's live [ATmega328PB product page](https://www.microchip.com/en-us/product/ATmega328PB)
  returned **Status: In Production** in the real Chrome profile via
  `~/.hermes/scripts/mc-live-status.py ATMEGA328PB`.
- Microchip's [ATmega328PB Xplained Mini page](https://www.microchip.com/en-us/development-tool/ATMEGA328PB-XMINI)
  identifies the exact evaluation kit and links its current user guide and schematics.
- The live [DigiKey listing](https://www.digikey.com/en/products/detail/microchip-technology/ATMEGA328PB-XMINI/5338500)
  identified manufacturer part `ATMEGA328PB-XMINI`, marked it Active, and displayed
  **In-Stock: 185**, an **Add to Cart** option, and a unit price of **$16.11**.

Microchip's board guide maps yellow user LED D200 to PB5 and routes the target
USART0 pins PD0/PD1 through the mEDBG for its CDC COM port ([LED path](https://onlinedocs.microchip.com/oxy/GUID-FC2A0384-AC9D-45B4-951E-5C0DEFE8B2E9-en-US-5/GUID-583320E4-E4C8-43FB-AFD1-CB9AAA296CDF.html),
[CDC UART path](https://onlinedocs.microchip.com/oxy/GUID-FC2A0384-AC9D-45B4-951E-5C0DEFE8B2E9-en-US-5/GUID-B1413EC6-A511-412A-88EF-934D43813A56.html)).
The twin observes USART0 at a host-console connector; it does not simulate mEDBG
or USB. This profile claims only the LED and UART behavior exercised by the smoke.
