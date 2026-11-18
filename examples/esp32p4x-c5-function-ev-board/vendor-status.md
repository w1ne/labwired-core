# ESP32-P4X-C5-Function-EV-Board and BME280 status

Checked live on 2026-11-18.

Espressif's current [board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32p4/esp32-p4x-c5-function-ev-board/user_guide.html)
lists `ESP32-P4X-C5-Function-EV-Board` as the available part number, documents
retail orders, and maps J1 pins 3 and 5 to GPIO7 and GPIO8. Espressif's
[revision-change notice](https://documentation.espressif.com/en/PCN202600801_ESP32-P4_Chip_Revision_v3.2_Upgrade_Chip_Revision_v1.3_Demand_Collection_and_EOL_Plan_Description.html)
plans EOL for the previous non-X ESP32-P4NRW16/32 parts and the previous
ESP32-P4-Function-EV-Board, and recommends the v3.2/X-suffix successors. This
onboarding selects the `P4X` board variant, aligned with the current revision;
it does not target the previous non-X parts. Espressif lists ESP32-P4 in its
[12-year longevity commitment](https://www.espressif.com/en/products/longevity-commitment).

Bosch's current [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
continues to list the sensor and states a 10-year delivery commitment. As a
buyable breakout option, Adafruit's live
[BME280 STEMMA QT listing](https://www.adafruit.com/product/2652) showed the
product at $14.95 with an Add to Cart control. The breakout is the external
device represented by this system smoke; the model uses its BME280 I2C behavior.
