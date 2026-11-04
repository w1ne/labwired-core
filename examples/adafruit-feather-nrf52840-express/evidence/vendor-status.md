# Live vendor and purchase status: Adafruit Feather nRF52840 Express

Checked 2026-11-04.

- Adafruit product 4062 identifies the board as the Feather nRF52840 Express.
  Adafruit's own product page currently reports **Out of stock** and links to
  DigiKey as a purchase source.
- DigiKey's listing for Adafruit product 4062 / DigiKey part **1528-2828-ND**
  reports **Part Status: Active**, **729 in stock**, and an enabled Add to Cart
  control at **$24.95**. The distributor listing is currently purchasable even
  though Adafruit's direct stock is depleted.

## Board pin references

- Adafruit's official Arduino variant maps Feather D0/TX to P0.25 and D1/RX
  to P0.24 in [`variant.cpp`](https://github.com/adafruit/Adafruit_nRF52_Arduino/blob/master/variants/feather_nrf52840_express/variant.cpp).
- The official [`variant.h`](https://github.com/adafruit/Adafruit_nRF52_Arduino/blob/master/variants/feather_nrf52840_express/variant.h)
  maps `PIN_SERIAL1_TX` to D0 and `PIN_SERIAL1_RX` to D1.
- The Adafruit product listing identifies product 4062 and its nRF52840 MCU:
  [Adafruit product page](https://www.adafruit.com/product/4062).
- [DigiKey product listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/4062/9843410)
  is the live stock/status source used for purchaseability.
