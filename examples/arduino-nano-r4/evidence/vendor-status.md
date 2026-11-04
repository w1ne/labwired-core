# Live vendor status — Arduino Nano R4

Checked 2026-11-04 UTC.

- Arduino's [official Nano R4 store page](https://store.arduino.cc/collections/nano-family/products/nano-r4)
  lists SKU **ABX00142**, shows **€13.40**, and offers **Add to cart**. This is
  direct manufacturer orderability evidence for the board.
- Renesas' [live product page for R7FA4M1AB3CFM#HA0](https://www.renesas.com/en/products/ra4m1/part-details/r7fa4m1ab3cfm-ha0)
  identifies the MCU used in Arduino's board schematic and marks it **Active**.
- Arduino's [Nano R4 datasheet](https://docs.arduino.cc/resources/datasheets/ABX00142-datasheet.pdf)
  names the RA4M1, maps the on-board `LED_BUILTIN` to P204, and its schematic
  shows P204 driving DL2 through R9 to ground (active-high).
- Arduino's live [NANOR4 variant source](https://github.com/arduino/ArduinoCore-renesas/blob/main/variants/NANOR4/variant.cpp)
  maps pin 22 (`LED_BUILTIN`) to P204.

The board is currently purchasable from Arduino and its MCU lifecycle is
Active. The simulated profile only claims the P204 GPIO smoke; it does not
claim the unmodeled USB, I²C, SPI, ADC, DAC, CAN, timer, RGB LED, or power paths.
