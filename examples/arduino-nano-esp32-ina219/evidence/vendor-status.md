# Live vendor and availability check

Checked 2026-11-10 UTC for the Arduino Nano ESP32 (ABX00083 with headers) and
the INA219 current-sensor breakout used by this example.

- Arduino's [official Nano ESP32 store listing](https://store-usa.arduino.cc/products/nano-esp32-with-headers)
  identifies SKU **ABX00083** and says **“in stock, ready to be shipped”**.
  The [DigiKey ABX00083 page](https://www.digikey.com/en/products/detail/arduino/ABX00083/21219775)
  lists the board as **Active** and showed **2,305 in stock** when checked.
- Espressif's [ESP32-S3-DevKitC-1 page](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  lists current ESP32-S3 DevKitC-1 variants and links the N16R8 variant to
  Mouser. The Nano uses an ESP32-S3 module; the Nano board listing remains the
  exact hardware identity for this example.
- TI's [INA219 product page](https://www.ti.com/product/INA219) shows lifecycle
  status **ACTIVE**. The page describes its 0–26 V bus range and I²C/SMBus
  interface. TI's `INA219BIDCNRG4` page was out of stock on this check, so this
  example uses a buyable assembled breakout rather than claiming TI factory
  stock for that orderable suffix.
- Adafruit's [product 904 listing](https://www.adafruit.com/product/904) showed
  **In stock** at **$9.95**, with **Add to Cart**. It identifies the INA219A/B
  variants as interchangeable for the breakout design and gives the default
  I²C address as `0x40`.

This check found no NRND or obsolete designation for the modeled INA219
family. The evidence records a buyable board and assembled breakout alongside
the TI family lifecycle page; availability can change after this date.
