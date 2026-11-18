# Raspberry Pi Pico 2 live status and availability

Checked live on **2026-11-17 UTC** for the non-wireless Raspberry Pi Pico 2,
manufacturer SKU **SC1631**.

- Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
  says **“Available now from $5”**, identifies RP2350, and states the Pico 2
  series will remain in production until at least **January 2040**.
- DigiKey's [SC1631 listing](https://www.digikey.com/en/products/detail/raspberry-pi/SC1631/24627136)
  identifies the item as a Raspberry Pi Pico 2 RP2350 board, marks its part
  status **Active**, and showed **20,546 in stock** at the check. Its cut-tape
  listing showed $6.12 each and an average 1–3 day ship time.
- Raspberry Pi's [official Pico 2 pinout](https://pip-assets.raspberrypi.com/categories/1005-raspberry-pi-pico-2/documents/RP-008301-DS-1-Pico-2-Pinout.pdf)
  maps header pin 6 / GP4 to I2C0 SDA and pin 7 / GP5 to I2C0 SCL. The live
  [Pico-series documentation](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html)
  identifies Pico 2 as an RP2350 board and lists non-wireless variants with
  and without pre-soldered headers.

The board and RP2350 are current and buyable; this profile only models the
RP2350 firmware path, UART0 output, I2C0 transaction, and attached BME280 ID
register. It does not model USB, boot ROM, or the non-wireless board's supply
behavior. The availability check is a dated observation, not a future stock
guarantee.
