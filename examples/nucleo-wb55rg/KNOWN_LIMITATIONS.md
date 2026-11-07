# NUCLEO-WB55RG model limits

- The smoke covers the STM32WB55 Cortex-M4 application core, GPIO user LEDs,
  and USART1 register output only.
- The Cortex-M0+ radio core, 2.4 GHz transceiver, BLE, Thread, Zigbee, and
  802.15.4 stacks are omitted. The system does not substitute zero-answering
  stubs for those paths.
- The smoke does not exercise the user buttons, Arduino shields, or other
  peripherals in the STM32WB55 chip descriptor.
- ST-LINK programming/debug, USB devices, and the physical USART-to-USB VCP
  bridge are not simulated. UART output is captured at USART1 in the target
  model.
- LPUART1 on PA2/PA3 is exposed at the Arduino D1/D0 header; it is not the
  default ST-LINK VCP connection used by this smoke.
- The model is simulator-derived and has not been compared with a physical
  NUCLEO-WB55RG run.
