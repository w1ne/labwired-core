# Live vendor availability — ESP32-P4X-Function-EV-Board

Checked live on 2026-11-06:

- Espressif's [current board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32p4/esp32-p4x-function-ev-board/user_guide.html)
  lists the exact `ESP32-P4X-Function-EV-Board` SKU and describes it as using
  ESP32-P4 revision v3.1 or later. This is the refreshed board SKU; the guide
  distinguishes it from the older ESP32-P4-Function-EV-Board.
- The same guide maps **GPIO8 to J1 pin 5**. The profile uses only that header
  signal.
- Bürklin's [live exact-SKU listing](https://www.buerklin.com/en/p/espressif/microcontroller-development-boards/esp32-p4x-function-ev-board/60S1402/)
  showed **207 pcs available in 5 days if ordered today**, with an add-to-cart
  option. This is a current buyability check; stock and delivery estimates can
  change.
- ESP-IDF's current P4 `hw_ver3` [GPIO register header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32p4/register/hw_ver3/soc/gpio_reg.h),
  [IO_MUX register header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32p4/register/hw_ver3/soc/io_mux_reg.h),
  and [register base header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32p4/register/hw_ver3/soc/reg_base.h)
  provide the GPIO8 register offsets and peripheral bases used by this profile.

The `labwired_part` lookup for `ESP32-P4X-Function-EV-Board` returned
`NOT_FOUND` on this date. The new model is therefore not represented in that
catalog yet; this entry records the external primary sources used for the
focused implementation.
