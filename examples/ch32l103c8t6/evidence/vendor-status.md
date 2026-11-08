# CH32L103C8T6 — live availability

Checked live on 2026-11-08 UTC, in a real Chrome (the WCH and LCSC pages are
JavaScript-rendered and return only a shell to an HTTP client).

- WCH's [CH32L103 product page](https://www.wch-ic.com/products/CH32L103.html)
  ("PDUSB low-power microcontroller CH32L103"), filed under MCU/SoC >
  RISC-V featured application, lists it as a current product:
  "RISC-V4C processor, up to 96MHz system main frequency", "20KB SRAM, 62KB
  Flash", "4 groups of USART", "2 I2C interface", "2 SPI interface",
  "1 CAN interface (2.0B active)", "USB PD and Type C controller and PHY",
  "Package form: LQFP48, QFN32, QSOP28, QFN20, TSSOP20", with the
  CH32L103DS0.PDF datasheet and CH32L103RM.PDF reference manual. WCH
  publishes no "Active"/"NRND" lifecycle label (see the CH32V003/V006/V203/
  V307/X035 receipts), so this receipt cites the product page plus stock.
- LCSC [CH32L103C8T6, C22398291](https://www.lcsc.com/product-detail/C22398291.html):
  "In-Stock: 513", "513 In stock, ships now", 1+ $0.7819, 10+ $0.6308,
  30+ $0.5568, LQFP-48(7x7). No "Not recommended for new" or discontinued
  label on the listing.

The LQFP48 C8T6 is the package WCH's Arduino core ships its only L10x
variant for (variants/CH32L10x/CH32L103C8T6) and ch32fun's default
`TARGET_MCU_PACKAGE` for `TARGET_MCU=CH32L103`.
