# Live vendor status and availability

Checked 2026-11-06 UTC for the Seeed Studio XIAO ESP32-C6 and the ESP32-C6FH4
SoC used by this board.

- Seeed's [official product listing](https://www.seeedstudio.com/Seeed-Studio-XIAO-ESP32C6-p-5884.html)
  identifies SKU **113991254**, shows **$5.20**, **In stock**, and an **Add to
  Cart** control.
- Seeed's [official XIAO ESP32-C6 pin map](https://wiki.seeedstudio.com/xiao_esp32c6_getting_started/)
  maps **D4/SDA to GPIO22** and **D5/SCL to GPIO23**. The same guide identifies
  the on-board SoC as ESP32-C6 and lists 4 MB flash.
- Espressif's [current ESP32-C6 datasheet v1.5](https://documentation.espressif.com/esp32-c6_datasheet_en.html)
  includes the ESP32-C6FH4 4 MB variant and defines v1.0-and-later datasheets
  as the official release for products in mass production. The current v1.5
  datasheet has no NRND or EOL watermark.
- Adafruit's [official BME280 breakout listing](https://www.adafruit.com/product/2652)
  lists PID **2652** at **$14.95**, marks it **In stock**, and offers **Add to
  Cart**. DigiKey's [manufacturer part listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
  identifies the device as **Part Status: Active**.

The complete board and sensor breakout are orderable today. The SoC evidence
is for the C6 product family; it is not a separate lifecycle statement for the
Seeed board SKU. The simulation uses the BME280 breakout at I2C address
`0x77`; the board listing does not bundle that breakout.
