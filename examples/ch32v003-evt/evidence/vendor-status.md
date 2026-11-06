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

## CH32V003EVT + SHT31 I2C path

Rechecked live on 2026-11-06 UTC for the board and sensor used by the new I2C
smoke.

- WCH's live [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html)
  continues to list one I2C interface and links the current
  [CH32V003 reference manual](https://www.wch-ic.com/downloads/CH32V003RM_PDF.html)
  (v1.9, dated 2025-10-09). WCH does not publish an “Active”/“NRND” label on
  the product page, so no formal lifecycle label is inferred.
- OLIMEX's live [WCH-CH32V003EVT listing](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/)
  identifies the board MCU as CH32V003F4P6 and showed €3.95, “In Stock”, and
  “Add to basket”. The catalog also showed the CH32V003F4P6 ten-pack in stock.
- Sensirion's live [SHT31-DIS-B product page](https://sensirion.com/products/catalog/SHT31-DIS-B)
  lists the standard digital SHT31 humidity/temperature sensor in the SHT3x
  family; the live [SHT3x catalog](https://sensirion.com/products/catalog/series/sht3x)
  reports 12 Active products and 1 End of life product in the series.
- The ready-to-wire [Adafruit SHT31-D breakout](https://www.adafruit.com/product/2857)
  page showed $13.95 and “In stock” with an Add to Cart control. This is the
  buyable sensor assembly used by the example; the model assumes its default
  I2C address 0x44.
- WCH's official [CH32 Arduino core](https://github.com/openwch/arduino_core_ch32)
  lists CH32V003F4P and I2C master support since core version 1.0.2. Its
  [variant pin map](https://github.com/openwch/arduino_core_ch32/blob/main/variants/CH32V00x/CH32V003F4/PeripheralPins.c)
  maps SDA to PC1 and SCL to PC2 by default.
