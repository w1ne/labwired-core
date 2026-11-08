# Live vendor status — WeAct STM32F411CEU6 Black Pill

Checked 2026-11-04 UTC.

The LabWired Part Knowledge lookup returned `NOT_FOUND` for the exact
`STM32F411CEU6` part, so no Part Knowledge facts are claimed here; the status
and availability below are recorded from the live ST and seller pages.

- ST's [STM32F411CE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f411ce.html)
  lists `STM32F411CEU6` as **Active** and says the product is in volume
  production. ST reported no distributor inventory for the bare `CEU6` when
  checked; this record does not claim direct chip stock.
- WeAct's [MiniSTM32F4x1 product literature](https://github.com/WeActStudio/WeActStudio.MiniSTM32F4x1)
  lists STM32F411CEU6 as a supported, current product and documents the C13 LED.
  The [official WeAct Store listing](https://www.aliexpress.com/item/1005001456186625.html)
  identifies its STM32F411CEU6 BlackPill as V3.1; its public product description
  did not expose a live quantity in the captured page.
- LaskaKit's [live board listing](https://www.laskakit.cz/en/blackpill-arm-stm32-stm32f411ceu6-vyvojova-deska/)
  showed the WeAct STM32F411CEU6 Black Pill (SKU `LA100021`) **In stock (42 pcs)**
  with an Add to cart control and delivery dated `04/11/2026` when checked.
  That listing identifies the user LED on PC13 and links its V3.0 schematic.

The candidate remains purchasable as a complete board today, while ST's status
is active. The board smoke covers only the PC13 LED path; the 25 MHz crystal,
USB-C, PA0 key, optional SPI flash, and unrelated MCU peripherals are outside
its descriptor and are not claimed as working.
