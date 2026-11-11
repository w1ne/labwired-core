# CH32V003F4P6-EVT-R0-1v1 and CH32V003 availability

Checked live 2026-11-11.

- WCH's live [CH32V003 product page](https://www.wch-ic.com/products/CH32V003.html)
  lists the family, its 16 KiB flash / 2 KiB SRAM / 48 MHz QingKe V2A
  specifications, and links the current datasheet, reference manual, and EVT
  board package. WCH's page does not expose an explicit lifecycle field.
- The [WCH CH32V00x Evaluation Board Reference](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/resources/CH32V00x-Evaluation-Board-Reference-EN.pdf)
  identifies the CH32V003F4P6 evaluation board and its LED header P4. The USB
  connector is power-only; LED1 must be jumpered to a GPIO.
- The [CH32V003 EVT board wiring guide](https://ch405labs.net/ch32v003_intro/)
  describes the LED anodes connected through resistors to VCC and their
  cathodes connected to LED1/LED2, so the mapped LED is active-low.
- [Olimex's exact WCH-CH32V003EVT listing](https://www.olimex.com/Products/RISC-V/WCH-CH32V003EVT/)
  showed the board in stock at €3.95.
- [DigiKey's exact WCH-CH32V003EVT listing](https://www.digikey.com/en/products/detail/olimex-ltd/WCH-CH32V003EVT/29208930)
  showed Part Status **Active**, 11 units in stock, and a $4.85 unit price.
  This is live authorized-distributor status and stock evidence for the exact
  evaluation board; WCH itself does not publish a lifecycle label on its page.
- [Bosch BME280](https://www.digikey.com/en/products/detail/bosch-sensortec/BME280/6136306)
  showed Part Status **Active** and 6,301 units in stock. The physical example
  breakout, [Adafruit product 2652](https://www.adafruit.com/product/2652),
  showed **In stock** at $14.95; its [pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts)
  lists 0x77 as the default I2C address.

Demand rank: **Tier C — WCH vendor bet**; CH32V003 has no numeric percentage in
the survey table. The modelled board supports reset-clock GPIO, polled USART1,
and the default-route I2C1 master path (PC1=SDA, PC2=SCL) used by the BME280
smoke. It does not claim USB data, PFIC interrupts, DMA, remapped I2C pins, or
other on-chip peripherals.
