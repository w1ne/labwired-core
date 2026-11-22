# Live vendor status — Arduino Nano 33 BLE Rev2 + Adafruit BME280 breakout

Checked live on 2026-11-20 UTC (01:33 to 01:45).

- Arduino store, https://store.arduino.cc/products/nano-33-ble-rev2, read in
  Chrome (the page answers HTTP 429 to curl) → `Arduino Nano 33 BLE Rev2`,
  `SKU ABX00071`, price EUR 25,90, `SOLD OUT`, `Email me when available`.
  The US store, https://store-usa.arduino.cc/products/nano-33-ble-rev2, shows
  the same SKU at USD 23.10, also `SOLD OUT`. Neither page marks the board
  retired.
- Arduino documentation, https://docs.arduino.cc/hardware/nano-33-ble-rev2/:
  current hardware page with a `Buy now` link; no retired or end-of-life
  wording.
- Distributor stock, https://www.findchips.com/search/ABX00071 → Avnet
  Silica, `Authorized Distributor`, manufacturer part `ABX00071`, 100 in
  stock. This is the evidence that the board can be bought today. DigiKey,
  Mouser and TME refused the automated read and were not checked.
- Nordic, https://www.nordicsemi.com/Products/nRF52840: current product page
  for the nRF52840 SoC; no NRND or obsolete wording. The page shows no
  explicit status label.
- Adafruit BME280 breakout, https://www.adafruit.com/product/2652: last read
  on 2026-11-19 15:58 UTC for `examples/arduino-nano-every-bme280` →
  schema.org availability `InStock`, price 14.95 (USD). Not read again today.

The first Nano 33 BLE (ABX00030, LSM9DS1 IMU) was not checked. The sketch and
the PlatformIO board `nano33ble` are the same for both revisions; the two
differ in the IMU and the power supply, which this example does not use.

Not verified, and not modelled: the on-board BMI270 and BMM150. No IMU model
was added.
