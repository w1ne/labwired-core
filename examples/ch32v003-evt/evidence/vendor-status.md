# CH32V003F4P6 live availability check

Checked 2026-11-05 UTC.

- WCH's live [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html)
  lists the CH32V003 family and links its current datasheet and reference
  manual. The page does not expose a lifecycle label such as “Active”, “NRND”,
  or “Obsolete”, so this check does not claim an explicit WCH lifecycle status.
- WCH's live [download catalog](https://www.wch-ic.com/downloads) lists
  `CH32V003RM.PDF`, version 1.9, uploaded 2025-10-09, and the CH32V003 datasheet.
- The current [OLIMEX WCH/RISC-V catalog](https://www.olimex.com/Products/RISC-V/)
  lists the WCH-CH32V003EVT board at €3.95 as “In Stock” and the
  CH32V003F4P6 in a 10-piece pack at €2.50 as “In Stock”. This establishes a
  current purchasable path for both the selected evaluation board and MCU; it
  is a reseller availability check, not a WCH lifecycle declaration.
- WCH's [CH32V00x evaluation-board reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
  identifies the board's MCU as CH32V003F4P6 and says its LEDs connect to the
  MCU through the LED-row pins (P4). It also identifies the separate debug
  interface on PD1.
- WCH's official [GPIO_Toggle sample](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/GPIO/GPIO_Toggle/User/main.c)
  configures PD0 as a push-pull output and enables GPIOD through RCC. The
  descriptor covers that smoke path only.
