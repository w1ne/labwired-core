# nRF52840-DK live availability check

Checked 2026-11-04 11:36 UTC.

- Nordic's live [nRF52840 DK product page](https://www.nordicsemi.com/Products/Development-hardware/nRF52840-DK)
  still presents the DK as a current development kit for the nRF52840 SoC and
  lists its user-programmable LEDs and buttons.
- DigiKey's live [NRF52840-DK listing](https://www.digikey.dk/en/products/detail/nordic-semiconductor-asa/NRF52840-DK/8593726)
  identifies the manufacturer product number as `NRF52840-DK`, reports **Part
  Status: Active**, **In-Stock: 132**, and offers **Add to Cart**. Inventory and
  regional pricing can change after this check.
- Nordic's [nRF52840 DK user guide](https://infocenter.nordicsemi.com/pdf/nRF52840_DK_User_Guide_v1.4.1.pdf)
  identifies physical LED1 on P0.13. The live [Zephyr board description](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/nordic/nrf52840dk/nrf52840dk_nrf52840.dts)
  calls it the active-low `led0` board alias. The smoke uses only that GPIO
  path; other board features remain outside the focused smoke model.
