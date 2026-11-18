# Live vendor status — Adafruit ESP32 Feather V2 + BME280

Checked live on 2026-11-11 UTC. Adafruit's [Feather V2 product listing, PID
5400](https://www.adafruit.com/product/5400) listed the PCB-antenna variant
for **$19.95**, marked it **In stock**, and showed **Add to Cart**. The w.FL and
assembled-header options were out of stock; this coverage targets the
in-stock PCB-antenna version.

Adafruit's [BME280 STEMMA QT product listing, PID
2652](https://www.adafruit.com/product/2652) listed the breakout for **$14.95**,
marked it **In stock**, and offered **Add to Cart**. This verifies a buyable
assembled breakout carrying a Bosch BME280; it does not claim the bare Bosch
sensor was in stock at every distributor.

Bosch's [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
still describes the part, lists I2C/SPI interfaces, and displays a 10-year
delivery statement. Its distributor panel showed zero inventory at the listed
major distributors when checked. Adafruit's stocked module is the exact
buyable hardware used in this board-level smoke; neither vendor page labeled
the BME280 NRND or obsolete.

Adafruit's [Feather V2 guide](https://learn.adafruit.com/adafruit-esp32-feather-v2)
identifies the red user LED as pin 13 / GPIO13 and `LED_BUILTIN`. It identifies
the NeoPixel on GPIO0 and the shared NeoPixel/STEMMA QT power switch on GPIO2;
the guide says GPIO2 must be high for the QT port to work and that the Feather
has no I2C pull-ups. The Feather I2C pins are SDA=GPIO22 and SCL=GPIO20.

Adafruit's [BME280 breakout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/downloads)
links the STEMMA QT schematic, which includes two 10 kΩ I2C pull-ups. Its
[product listing](https://www.adafruit.com/product/2652) describes the
breakout's I2C/SPI interfaces and STEMMA QT connector. Bosch's [BME280
datasheet](https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds002.pdf)
documents the sensor's I2C/SPI digital interfaces.
