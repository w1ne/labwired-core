# STM32F103C8T6 Blue Pill availability and lifecycle

## Re-checked live on 2026-11-25 (PWM round)

- ST's [STM32F103C8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103c8.html)
  renders status **ACTIVE** with the summary "Mainstream Performance line, Arm
  Cortex-M3 MCU with 64 Kbytes of Flash memory, 72 MHz CPU, motor control, USB
  and CAN". The site answers no plain HTTP client, so the page was read through
  the real Chrome session.
- ElectroPeak's [Blue Pill listing](https://electropeak.com/stm32f103c8t6-arm-board)
  showed **In Stock**, SKU `MIC-02-012`, EUR 2.42, title
  "STM32F103C8T6 Blue Pill ARM Development Board".
- Neither page carried an NRND or EOL notice.

## Checked live on 2026-11-12

- ST's [STM32F103C8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103c8.html)
  lists order code `STM32F103C8T6` as **Active** and describes the product as in
  volume production. ST's page did not report distributor inventory when
  checked; it offered direct order and free-sample links.
- ElectroPeak's [STM32F103C8T6 Blue Pill listing](https://electropeak.com/stm32f103c8t6-arm-board)
  showed **In stock** and identifies the board MCU as `STM32F103C8T6`, with
  64 KB Flash and 20 KB SRAM.
- The reference [Blue Pill schematic](https://stm32-base.org/assets/pdf/boards/original-schematic-STM32F103C8T6-Blue_Pill.pdf)
  connects its user LED through a resistor to PC13. Blue Pill clones vary; this
  profile represents that reference PC13 LED arrangement and the genuine ST
  STM32F103C8T6 silicon model. The reseller listing does not establish the
  authenticity of every shipped board.

The board page is a retailer listing for a third-party minimum-system board,
not an ST evaluation board. The simulator example therefore claims only the
PC13 GPIO LED and USART1 register path; it does not claim USB, onboard debug,
physical serial capture, or physical-board validation.
