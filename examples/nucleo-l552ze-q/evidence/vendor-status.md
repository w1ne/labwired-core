# NUCLEO-L552ZE-Q live vendor status

Checked live in Chrome on 2026-11-04.

- ST's [official eStore product listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-l552ze-q.html)
  identified **NUCLEO-L552ZE-Q** as **ACTIVE** and **In Stock**, showed a
  **$39.10** price, and offered **Add to Cart**.
- ST's [STM32L552ZE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32l552ze.html)
  lists the board's exact **STM32L552ZET6Q** MCU as **Active** and says the
  product is in volume production.

The board is orderable from its manufacturer today, and its MCU is not NRND or
obsolete. The status check does not imply that every peripheral named by the
full chip descriptor is modelled or exercised. The onboarding smoke covers
only the board's existing GPIO LED/button and LPUART paths; the remaining
profile limits are documented in [`../VALIDATION.md`](../VALIDATION.md).
