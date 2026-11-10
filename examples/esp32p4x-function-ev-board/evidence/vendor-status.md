# Vendor and register sources — ESP32-P4X Function EV Board

Checked 2026-11-10:

- Espressif's [current board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32p4/esp32-p4x-function-ev-board/user_guide.html)
  identifies the exact `ESP32-P4X-Function-EV-Board` SKU and says its P4 chip
  revision is v3.1 or later. It distinguishes this refreshed board from the
  older ESP32-P4-Function-EV-Board.
- The same board guide lists GPIO8 on J1 pin 5, GPIO21 on J1 pin 11, and GPIO22
  on J1 pin 12.
- Bürklin's [exact-SKU listing](https://www.buerklin.com/en/p/espressif/microcontroller-development-boards/esp32-p4x-function-ev-board/60S1402/)
  returned `Available in 5 Days: 207 pcs. - if ordered today` and an `Add to
  Cart` option during this round's page fetch. The web retrieval metadata marks
  the page content as crawled 6 days earlier, so the stock count may be cached;
  availability is subject to prior sale and this is not an order reservation.
- Espressif's [ESP32-P4 Technical Reference Manual](https://documentation.espressif.com/esp32-p4_technical_reference_manual_en.pdf)
  documents I2C0 at `0x500C4000` and its interrupt source 44. Its command-list
  register layout matches the modeled ESP32-S3 I2C block used by the focused
  implementation.
- Espressif's ESP-IDF P4 [GPIO register header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32p4/register/hw_ver3/soc/gpio_reg.h)
  and [IO_MUX register header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32p4/register/hw_ver3/soc/io_mux_reg.h)
  provide the GPIO8 offsets used by the output smoke.

The `labwired_part` lookup for this board returned `NOT_FOUND`; the links above
are the source evidence for the board mapping and focused model.
