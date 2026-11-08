# Vendor status — Arduino UNO R4 WiFi

Checked 2026-11-03 on the live vendor product pages:

- Arduino's [UNO R4 WiFi store page](https://store.arduino.cc/products/uno-r4-wifi) identifies SKU **ABX00087**, lists it at **€30.50**, and currently offers **Add to cart**. Its product description names the RA4M1 and ESP32-S3 modules.
- The board's main MCU is Renesas **R7FA4M1AB3CFM#AA0**. Renesas's [live part page](https://www.renesas.com/en/products/ra4m1/part-details/r7fa4m1ab3cfm-aa0) lists it as **Active**. At check time its authorized distributor table showed **1,526 at DigiKey** and **339 at Mouser**. Renesas's direct order widget was set to 1,000 units and reported that quantity exceeded available inventory, so distributor stock is the purchase evidence for the MCU.

The ESP32-S3 is present on the physical board but is outside this model profile; this round does not add a wireless-module model.

## Re-check 2026-11-08 (stock Arduino matrix row)

- Arduino store page (fetched live, product JSON-LD): SKU **ABX00087**,
  `availability: http://schema.org/InStock`, EUR **22.88**.
- Renesas part page for **R7FA4M1AB3CFM#AA0**: status **Active**.

## Re-check 2026-11-08 (matrix L4, SPI0)

- Arduino store page for ABX00087 (fetched live): JSON-LD
  `availability: http://schema.org/InStock`, EUR **22.88**.
- Renesas part page for **R7FA4M1AB3CFM#AA0**: status **Active**.
