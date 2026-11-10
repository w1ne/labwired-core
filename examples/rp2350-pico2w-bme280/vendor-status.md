# Live availability check — 2026-11-10, approximately 23:37 UTC

- Raspberry Pi's [Pico 2 W product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/?variant=pico-2-w) lists the W variant with Wi-Fi and Bluetooth and provides an active **Buy now** path. The page says the Pico 2 series remains in production until at least January 2040.
- The [Pico 2 W datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-2-pico-2-w-datasheet.pdf) gives a product-specific availability guarantee through at least January 2028. This is still in production on the check date; I record this nearer date rather than implying the series-wide 2040 date applies to the individual W SKU.
- Mouser's [Raspberry Pi SC1633 listing](https://www.mouser.com/en/ProductDetail/Raspberry-Pi/SC1633) identified the Pico 2 W and displayed **2,450 in stock**, available to ship immediately, quantity minimum 1.
- Infineon's [CYW43439 product page](https://www.infineon.com/part/CYW43439) reported **Product Status: active and preferred** and **in stock**. The chip is intentionally omitted from the simulator descriptor; no radio behavior is claimed.
- Adafruit's [BME280 breakout product 2652](https://www.adafruit.com/product/2652) showed **$14.95**, **In stock**, and **Add to Cart**.

## Scope notes

The Pico 2 W board is not obsolete or NRND on the check date. The profile only
models RP2350 I2C0 and the external BME280. It omits CYW43439, Wi-Fi, Bluetooth,
the WL_GPIO0 user LED, USB, board power circuitry, and electrical pad behavior.
