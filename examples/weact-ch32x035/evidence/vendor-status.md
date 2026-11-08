# CH32X035F8U6 / CH32X035C8T6 / WeAct CH32X035 core board — live availability

Checked live on 2026-11-08 UTC, in a real Chrome (the WCH and LCSC pages are
JavaScript-rendered and return only a shell to an HTTP client).

- WCH's [CH32X035 product page](https://www.wch-ic.com/products/CH32X035.html)
  ("USB+Type-C PD MCU CH32X035") lists the part as a current product:
  "RISC-V4C processor, up to 48MHz", "built-in USB and PD PHY", datasheet
  CH32X035DS0.PDF, reference manual CH32X035RM.PDF and the CH32X035EVT.ZIP
  routines. WCH publishes no "Active"/"NRND" lifecycle label (see the
  CH32V003/V203/V307 receipts), so this receipt cites the product page plus
  stock.
- LCSC [CH32X035F8U6, C42442062](https://www.lcsc.com/search?q=CH32X035F8U6):
  "1,360 In Stock", 1+ $0.5441, 1,000+ $0.2721, QFN-20-EP(3x3), "RISC-V4C 32
  Bit 48MHz".
- LCSC [CH32X035C8T6, C29779902](https://www.lcsc.com/product-detail/C29779902.html):
  "426 In stock, ships now", 1+ $0.7319, 1,000+ $0.3841, LQFP48.
- WeAct Studio's board repository,
  [WeActStudio.CH32X035CoreBoard](https://github.com/WeActStudio/WeActStudio.CH32X035CoreBoard),
  names the MCU "CH32X035F8U6" and sells it through weactstudio.aliexpress.com
  and weactstudio.taobao.com. Its own `Examples/01-Blink/User/main.c` drives
  GPIOB pin 12, the LED pad ch32fun's blink also names. The store listing
  itself was not read for stock.

Package bonding is from the CH32X035 datasheet V1.7 (CH32X035DS0_en.PDF in
the WeAct repository): QFN20 bonds PA0..PA7, PB0, PB1, PB3, PB11, PB12 and
PC14..PC19, so PC7 (ch32fun's exti_pin_change_isr input) exists only on the
larger packages such as the LQFP48 CH32X035C8T6.
