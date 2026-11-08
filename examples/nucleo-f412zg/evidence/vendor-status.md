# Live vendor status: NUCLEO-F412ZG

Checked 2026-11-04 UTC against STMicroelectronics' live product page and online
store.

- ST's [NUCLEO-F412ZG product page](https://www.st.com/en/evaluation-tools/nucleo-f412zg.html)
  lists the board as **Active** and says it is in volume production.
- ST's [online store listing](https://estore.st.com/en/nucleo-f412zg-cpn.html)
  showed **NUCLEO-F412ZG In Stock**, priced at **$30.52 each for quantities
  1–2**, with **Add to Cart** available.
- ST's [STM32F412ZG product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f412zg.html)
  lists the fitted **STM32F412ZGT6** as **Active** and “Product is in volume
  production.” The page also identifies the F412 family as Cortex-M4, up to
  1 MiB Flash, and 256 KiB SRAM.
- ST's [UM1974, STM32 Nucleo-144 boards](https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boardsstmicroelectronics.pdf)
  identifies NUCLEO-F412ZG's target as STM32F412ZGT6; §7.5 maps active-high
  blue user LED LD2 to PB7. The default ST-LINK VCP uses PD8/PD9, but that path
  is intentionally not exposed by this profile.
- ST's [STM32F412xE/G datasheet](https://www.st.com/resource/en/datasheet/stm32f412zg.pdf)
  and [RM0402 reference manual](https://www.st.com/resource/en/reference_manual/rm0402-stm32f412-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
  support the declared memory, Cortex-M4, F4 RCC, and GPIO register layout.

The board was Active and directly purchasable from ST at this check. Price and
inventory can change.
