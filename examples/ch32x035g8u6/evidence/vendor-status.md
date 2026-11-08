# CH32X035G8U6 live availability

Checked live on 2026-11-08 UTC, in a real Chrome (the WCH and LCSC pages are
JavaScript-rendered and return only a shell to an HTTP client).

- WCH's [CH32X035 product page](https://www.wch-ic.com/products/CH32X035.html)
  lists the part as a current product: "RISC-V4C processor, up to 48MHz",
  "Package forms: LQFP64M, LQFP48, QFN28, QSOP28, QFN20, TSSOP20", product
  manuals CH32X035DS0.PDF and CH32X035RM.PDF, CH32X035EVT.ZIP, and a "Get
  Sample" control. WCH publishes no "Active"/"NRND" lifecycle label (see the
  CH32V003/V203/V307 and CH32X035F8U6/C8T6 receipts), so this receipt cites
  the product page plus stock.
- LCSC [CH32X035G8U6, C7437027](https://www.lcsc.com/search?q=CH32X035G8U6):
  "8,787 In Stock", 1+ $0.5399, 10+ $0.4234, 500+ $0.2839, 1,000+ $0.2675,
  QFN-28(4X4), "RISC-V4C 32 Bit 48MHz", 62KB FLASH, full reel 6,000.

The G8U6 is the only CH32X035 variant WCH's Arduino core ships
(`variants/CH32X035/CH32X035G8U`), and PlatformIO's ch32v platform offers the
arduino framework only for `genericCH32X035G8U6` (and the G8U6 EVT/PD-monitor
boards).
