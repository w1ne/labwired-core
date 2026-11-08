# Live vendor and availability check

Checked 2026-11-06 UTC for the Arduino Nano ESP32 headered board and the
Adafruit BME280 breakout used by this profile.

- Arduino's [official store listing](https://store.arduino.cc/products/nano-esp32)
  identifies SKU **ABX00092**, names the u-blox NORA-W106 (ESP32-S3), lists
  **€14.26** on sale (**€20.40** list price), and offers **Add to cart**. The
  listing does not give the complete board a separate Active/NRND/EOL field.
- Espressif's live [Product Selector](https://products.espressif.com/product-selector)
  lists **ESP32-S3R8** as **Mass Production**. This is the module's SoC status;
  it is not a separate lifecycle classification for the Arduino board SKU.
- Arduino's [Nano ESP32 manual](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
  and [official full pinout](https://docs.arduino.cc/resources/pinouts/ABX00083-full-pinout.pdf)
  map **A4/SDA to GPIO11** and **A5/SCL to GPIO12**.
- Adafruit's [product 2652 listing](https://www.adafruit.com/product/2652)
  showed the BME280 STEMMA QT breakout **In stock** at **$14.95** with **Add to
  Cart**. The [DigiKey listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
  reports **Part Status: Active** and **2,343 in stock**.

The board is orderable today and the ESP32-S3R8 is in mass production. No
obsolete or NRND component is being added.

Demand rank: ESP32 family **#1** (38% reported MCU use); Arduino boards were
**37%** of reported project kits. This round adds the Nano ESP32's documented
A4/A5 I2C sensor path; it does not add a new MCU family or peripheral model.
