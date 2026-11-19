# ESP32-S3-LCD-EV-Board v1.5 and SUB2 touch status

Checked live on 2026-11-19.

Espressif's current [ESP32-S3-LCD-EV-Board v1.5 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-lcd-ev-board/user_guide.html)
lists the board with the SUB2 v1.5 3.95-inch, 480×480 touchscreen, names the
touch IC as FT5x06, and provides an active retail-order link to Espressif's
[online shop](https://www.aliexpress.us/item/3256804584611152.html). The same
current guide identifies the module as ESP32-S3-WROOM-1-N16R16V and assigns
GPIO47/GPIO48 to I²C SDA/SCL. No EOL or NRND label appears in the current
board guide or product listing.

Espressif's [live S3 development-board catalog](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
also lists the ESP32-S3-LCD-EV-Board and links retail channels including
AliExpress, DigiKey, and Mouser. The selected board and touchscreen are
current buyable products, not an archived variant.

The address `0x38` is defined by Espressif's current
[FT5x06 BSP header](https://github.com/espressif/esp-bsp/blob/master/components/lcd_touch/esp_lcd_touch_ft5x06/include/esp_lcd_touch_ft5x06.h).
Touch-register offsets and the packed point layout follow FocalTech's
[FT5x06 Linux driver](https://github.com/focaltech-systems/drivers-input-touchscreen-FTS_driver/blob/master/ft5x06.c).
