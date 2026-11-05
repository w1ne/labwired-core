# CH32V003F4P6 live availability check

Checked live on 2026-11-05 UTC for both the CH32V003F4P6 and CH32V003EVT.

- WCH's live [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html)
  still lists the family and links its current datasheet and reference manual.
  WCH does not expose an “Active”, “NRND”, or “Obsolete” label there, so this
  receipt does not claim a formal lifecycle classification.
- The live [OLIMEX WCH/RISC-V catalog](https://www.olimex.com/Products/RISC-V/)
  lists the WCH-CH32V003EVT at €3.95 as “In Stock” and CH32V003F4P6 (10-pack)
  at €2.50 as “In Stock”. The live [exact MCU listing](https://www.olimex.com/Products/RISC-V/WCH/SMD-IC-CH32V003F4P6/)
  also shows “In Stock” and an add-to-basket control. That establishes a current
  purchase path for both the selected MCU and evaluation board; it is reseller
  stock evidence, not a WCH lifecycle declaration.
- WCH's [CH32V00x evaluation-board reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
  identifies the board's MCU as CH32V003F4P6 and documents its LED-row pins and
  separate debug interface.
- WCH's official [GPIO_Toggle sample](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/GPIO/GPIO_Toggle/User/main.c)
  configures PD0 and enables GPIOD through RCC.
- WCH's official [USART_IAP sample](https://github.com/openwch/ch32v003/blob/main/EVT/EXAM/USART_IAP/CH32V003_IAP/User/iap.c)
  configures GPIOD and USART1, sets its baud register, and uses the PD5 TX
  route. The new descriptor adds only USART1 transmit; RX and other peripheral
  behavior remain outside its scope.
