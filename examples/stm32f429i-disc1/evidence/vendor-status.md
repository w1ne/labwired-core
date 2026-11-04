# Live vendor status — STM32F429I-DISC1

Checked live on 2026-11-04 against ST's product and store pages:

- [ST 32F429IDISCOVERY product page](https://www.st.com/en/evaluation-tools/32f429idiscovery.html)
  lists **STM32F429I-DISC1** as **Active** and says the product is in volume
  production. The page identifies STM32F429ZI as the board's MCU and states
  STM32F429I-DISC1 replaces the obsolete STM32F429I-DISCO order code.
- [ST eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-discovery-kits/32f429idiscovery.html)
  showed **Active**, **In Stock**, **$45.45** for quantity 1–2, and an enabled
  **Add to Cart** control.
- [ST STM32F429ZI product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f429zi.html)
  lists the MCU as **Active** and in volume production.

Board wiring references are ST UM1670 Rev 6 and the MB1075-F429I-E01 schematic,
listed in [`REQUIRED_DOCS.md`](../REQUIRED_DOCS.md). UM1670 identifies green
user LED LD3 on PG13; the ST board support package sets the pin high to turn
the LED on.
