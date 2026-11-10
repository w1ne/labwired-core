# CH32V003F4P6-EVT-R0-1v1 availability

Checked 2026-11-10.

- WCH's live [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html)
  lists the CH32V003 family, its 16 KiB flash / 2 KiB SRAM / 48 MHz QingKe V2A
  specifications, and links the current datasheet, reference manual, and EVT
  board package. The page does not expose an explicit lifecycle field, so this
  receipt does not claim a vendor lifecycle label.
- The [WCH CH32V00x Evaluation Board Reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
  identifies the CH32V003F4P6 evaluation board and its LED header P4. The USB
  connector is power-only; LED1 must be jumpered to a GPIO.
- The [CH32V003 EVT board wiring guide](https://ch405labs.net/ch32v003_intro/)
  describes the LED anodes connected through resistors to VCC and their
  cathodes connected to LED1/LED2, so the mapped LED is active-low.
- [Olimex's WCH-CH32V003EVT listing](https://www.olimex.com/Products/RISC-V/)
  showed the evaluation board in stock at €3.95 and the CH32V003F4P6 in stock
  (pack of 10 at €2.50) when checked. This is current distributor buyability
  evidence; it is not a lifecycle statement from WCH.

The modeled system is limited to the supported CH32V003 GPIO path and the
board's LED1 jumper. It does not claim USB data, PFIC interrupts, or the other
on-chip peripherals.
