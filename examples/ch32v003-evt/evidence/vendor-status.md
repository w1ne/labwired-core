# CH32V003F4P6 live availability check

## Community quickstart check — 2026-11-10 UTC

- WCH's [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html)
  remains the official family page. It is JavaScript-rendered and the live
  check did not expose a lifecycle label, so no “Active”, “NRND”, or
  “Obsolete” classification is inferred.
- Olimex's [exact CH32V003F4P6 listing](https://www.olimex.com/Products/RISC-V/WCH/SMD-IC-CH32V003F4P6/)
  identifies a WCH 32-bit RISC-V part in TSSOP-20 and showed a 10-piece pack
  for €2.50, In Stock, and Add to basket on 2026-11-10.
- Olimex's [exact WCH-CH32V003EVT board listing](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/)
  identifies the board's MCU as CH32V003F4P6 and showed €3.95, In Stock, and
  Add to basket on 2026-11-10. This is distributor availability, separate from
  WCH's lack of a published lifecycle label.
- The current quickstart GPIO smoke passed using the repo's Rust fixture; see
  [`community-quickstart-2026-11-10/`](community-quickstart-2026-11-10/). A
  rebuild of the separately pinned WCH C SDK example was attempted, but this
  host has no `riscv64-unknown-elf-gcc`; the earlier committed WCH SDK smoke
  pass remains at [`sdk-gpio-toggle-smoke/`](sdk-gpio-toggle-smoke/).

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

## CH32V003EVT clock bring-up and SysTick path

Rechecked live on 2026-11-06 UTC for the SysTick and WCH SDK smokes.

- WCH's [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html),
  read through a real Chrome session because plain HTTP clients get a
  JavaScript shell, still lists the family. It describes the QingKe RISC-V2A
  core with "Up to 48MHz system main frequency" and links CH32V003DS0.PDF and
  CH32V003RM.PDF. No Active/NRND/Obsolete label appears on the page, so no
  formal lifecycle class is claimed.
- OLIMEX's live [WCH-CH32V003EVT listing](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/)
  showed price 3.95 EUR, "In Stock", and "Add to basket".
- Register facts come from WCH's own SDK at openwch/ch32v003 commit
  `4e1b7437ada5d3ae8831a6f4b811a6320e4e23c7`: `core_riscv.h` (SysTick_Type at
  0xE000F000), `system_ch32v00x.c` (SystemInit, SetSysClockTo_48MHZ_HSI,
  SetSysClockTo_48MHz_HSE), `debug.c` (Delay_Init/Delay_Us/Delay_Ms), and
  `startup_ch32v00x.S` (`csrw 0x804`). The ch32v003fun delay idiom is from
  cnlohr/ch32v003fun `ch32fun.c` (`DelaySysTick`).
- The EVT schematic in the same SDK (`EVT/PUB/CH32V00xSCH.pdf`) shows a 24 MHz
  Y1 whose links R4/R5 are marked `NC(0R)`. That is why the unmodified example
  runs from the HSI at 24 MHz on the default board fit.

## CH32V003 TIM2 encoder and remap smoke

Rechecked live on 2026-11-07 UTC before extending TIM2 encoder and remap
coverage.

- WCH's live [CH32V003 product page](https://www.wch-ic.com/products/CH32V003)
  lists the QingKe V2A series and its 48 MHz, 2 KB SRAM, 16 KB flash, timer,
  USART, I2C, and SPI capabilities, and links the series datasheet, reference
  manual, and evaluation-board materials. The page does not publish an
  “Active”/“NRND”/“Obsolete” lifecycle label, so this receipt does not claim a
  formal classification.
- LCSC's live [CH32V003F4P6 listing](https://www.lcsc.com/product-detail/Microcontroller-Units-MCUs-MPUs-SOCs_WCH-Jiangsu-Qin-Heng-CH32V003F4P6_C5187096.html)
  identifies the WCH TSSOP-20 part and showed 15,435 units in stock, “ships
  now”, and an add-to-cart control at review time. That is current purchase
  evidence for the exact MCU; it is separate from WCH's lifecycle statement.
- WCH's product page points to its current CH32V003 evaluation-board archive,
  supporting the CH32V003EVT board profile used by these firmware smokes.
