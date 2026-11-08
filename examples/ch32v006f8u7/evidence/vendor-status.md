# CH32V006F8U7 — live availability

Checked live on 2026-11-08 UTC, in a real Chrome (the WCH and LCSC pages are
JavaScript-rendered and return only a shell to an HTTP client).

- WCH's [CH32V006 product page](https://www.wch-ic.com/products/CH32V006.html)
  ("64K-Flash Multi-function General-purpose MCU CH32V006"), filed under
  MCU/SoC > RISC-V general, lists it as a current product: "QingKe 32-bit
  RISC-V2C processor", "Up to 48MHz", "8KB SRAM", "2 USART interfaces",
  "Package: QFN32, QSOP24, QFN20, TSSOP20", with the CH32V006DS0.PDF
  datasheet and CH32V006EVT.ZIP routines. WCH publishes no "Active"/"NRND"
  lifecycle label (see the CH32V003/V203/V307/X035 receipts), so this receipt
  cites the product page plus stock.
- LCSC [CH32V006F8U7, C55112440](https://www.lcsc.com/search?q=CH32V006):
  "2,005 In Stock", 5+ $0.2791, 2,500+ $0.1482, 5,000+ $0.1397, QFN-20,
  "RISC-V 32 Bit 48MHz", 62KB flash.
- Not chosen: LCSC lists the QFN32 **CH32V006K8U6** (C48393430) as "Not
  available now" and "Not recommended for new". That is LCSC's listing
  label, not a WCH statement, but under the no-obsolete-parts rule this
  profile targets the in-stock F8U7 instead. The CH32V006DS0 model table
  lists the QFN32 part as K8U7.

Datasheet: CH32V006DS0.PDF (wch-ic.com/download/file?id=396). Its model
table puts the F8U7 at 62K flash, 8K SRAM, 18 GPIO, 2 USARTs, QFN20, 105 C;
Table 2-1-1 bonds PA1, PA2, PC0..PC7, PD0..PD7 on QFN20, which covers every
pin ch32fun's examples_x00x/blink (PD0, PD4, PD6, PC0) and dual_usart (PD5,
PD6, PD2, PD3) use.
