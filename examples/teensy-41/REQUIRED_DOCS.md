# Source documents — Teensy 4.1 D13 smoke

- PJRC [Teensy 4.1 product and pinout](https://www.pjrc.com/store/teensy41.html):
  board identity, LED pin, and product status/ordering link.
- NXP [i.MX RT1060 Processor Reference Manual](https://www.nxp.com/products/i.MX-RT1060):
  CCM, GPIO, and IOMUXC register definitions.
- NXP CMSIS-SVD `MIMXRT1062.svd`, vendor claim: register
  `IOMUXC.SW_MUX_CTL_PAD_GPIO_B0_03`, reset value `0x5`; MUX_MODE ALT5 selects
  GPIO2_IO03. Held SVD revision 1.0, SHA-256
  `4b017b09946d58262acb5757735b350aa9272dd08cf727af652ac9ea43e02d5a`, locator
  `$.peripherals.IOMUXC.registers.SW_MUX_CTL_PAD_GPIO_B0_03`.
- The exact on-board MCU is MIMXRT1062DVJ6B. See the live purchasing and
  lifecycle check in [`evidence/vendor-status.md`](evidence/vendor-status.md).
