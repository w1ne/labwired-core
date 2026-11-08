# Live vendor status: ST LIS2DUX12

Checked 2026-11-04 against ST's live product page and current distributor stock:

- [ST LIS2DUX12 product page](https://www.st.com/en/mems-and-sensors/lis2dux12.html)
  lists the LIS2DUX12 as **Active** and says it is in volume production.
- [ST eStore LIS2DUX12TR listing](https://estore.st.com/en/products/mems-and-sensors/accelerometers/lis2dux12.html)
  showed **Active** but **Out of Stock** (displayed price $1.89 at 500 units).
  [DigiKey's LIS2DUX12TR listing](https://www.digikey.com/en/products/detail/stmicroelectronics/LIS2DUX12TR/17627730)
  independently showed **Active**, **1,193 in stock**, and **Add to Cart**.
- [ST STEVAL-MKI238A evaluation adapter page](https://www.st.com/en/evaluation-tools/steval-mki238a.html)
  identifies the board as the LIS2DUX12 adapter. [DigiKey's listing](https://www.digikey.com/en/products/detail/stmicroelectronics/STEVAL-MKI238A/17879070)
  showed the adapter **Active** with 103 units in stock.
- The [official STEVAL-MKI238A schematic](https://www.st.com/resource/en/schematic_pack/steval-mki238a-schematic.pdf)
  identifies its sensor, rails, I2C signals, CS, and SA0 connections.
- The [LIS2DUX12 datasheet](https://www.st.com/resource/en/datasheet/lis2dux12.pdf)
  specifies WHO_AM_I `0x47`, I2C address `0x18` when SA0 is low, CTRL5 ODR/full
  scale fields, and signed little-endian XYZ outputs.

The selected sensor is active and purchasable from an authorized distributor
at the check time, even though ST's own store showed it out of stock. The
adapter was also in stock. This receipt is a dated snapshot; stock and price
can change.
