# Vendor status and board reference

Checked 2026-11-11:

- [ST STM32F103C8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103c8): status **Active**, product in volume production.
- [ST eStore STM32F103C8 page](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-174-cortex-174-mcus/stm32-mainstream-mcus/stm32f1-series/stm32f103/stm32f103c8.html): exact STM32F103C8T6 listing **Active** and **In Stock**, listed at $4.03 per 500 units when checked.
- [ElectroPeak Blue Pill listing](https://electropeak.com/stm32f103c8t6-arm-board): the STM32F103C8T6 Blue Pill board listing said **In stock** when checked.
- [Archived VccGND reference schematic](https://stm32-base.org/assets/pdf/boards/original-schematic-STM32F103C8T6-Blue_Pill.pdf): shows PC13 connected to the on-board LED through a resistor and PA9/PA10 exposed on the headers.

Blue Pill is a generic board layout sold by multiple vendors, not an ST-branded
evaluation kit. Copies vary, including the fitted MCU. This profile is for the
reference layout with a genuine STM32F103C8T6 and the cited PC13 LED wiring; the
listing and stock status are snapshots, not a guarantee of future supply.
