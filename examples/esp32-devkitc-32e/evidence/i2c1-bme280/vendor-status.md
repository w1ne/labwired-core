# Live lifecycle and availability — ESP32-DevKitC-32E + BME280

Checked 2026-11-10 UTC against current manufacturer documentation and
distributor listings.

- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists the exact `ESP32-DevKitC-32E` with an ESP32-WROOM-32E module and 4 MB
  flash. The live pin table identifies GPIO16 and GPIO17 as header pins and
  states that these pins are available on WROOM boards; WROVER boards reserve
  them for internal use.
- Mouser's [exact ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  showed 1,692 in stock, immediate shipment, minimum order quantity 1, and
  $10.21 each. The exact board is purchasable today.
- DigiKey's [ESP32-WROOM-32E-N4 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-WROOM-32E-N4/11613125)
  labels the module Active and showed 6,122 in stock. The module is the one
  fitted to this no-PSRAM, 4 MB board variant; the separate older
  ESP32-WROOM-32 is not used by this profile.
- Bosch Sensortec's [current BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  lists the sensor with a 10-year delivery commitment and current datasheet.
  Adafruit's [BME280 breakout, PID 2652](https://www.adafruit.com/product/2652)
  was listed in stock at $14.95; DigiKey's [same breakout](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
  labels it Active and showed 2,343 in stock.

Demand rank: **ESP32 family, rank 1** (38% measured MCU use). This round adds a
second-controller BME280 profile to the existing DevKitC-32E board. It is a
low-cost coverage extension, not a new board or MCU demand pick.

The committed local digital-twin smoke uses the classic ESP32 I2C1 command-list
registers at `0x3FF67000`, routes SDA=GPIO16 and SCL=GPIO17, and requires BME280
ID `0x60` plus raw temperature `0x082DC0`. It does not verify the Arduino
`Wire1` driver or physical electrical behavior. Vendor availability can change;
this record is a dated live check, not a hardware run.
