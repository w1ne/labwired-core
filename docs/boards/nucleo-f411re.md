# NUCLEO-F411RE

The NUCLEO-F411RE is an ST Nucleo-64 development board built around the
STM32F411RET6 (Cortex-M4F, 512 KiB flash, 128 KiB SRAM). ST currently lists
both the board and MCU as Active. See the
[live vendor record](../../examples/nucleo-f411re/evidence/vendor-status.md).

## Modeled board paths

The narrow board system maps the LD2 user LED to PA5 and the ST-LINK Virtual
COM Port to USART2 on PA2/PA3. The simulator smoke boots board-specific
firmware, sends `NUCLEO_F411RE_OK`, and toggles PA5. See the
[smoke README](../../examples/nucleo-f411re/README.md) and
[validation record](../../examples/nucleo-f411re/VALIDATION.md).

The profile declares only RCC, GPIOA, and USART2. No zero-answering
stubs are used; unlisted peripherals are unsupported. The USART2 transcript is
captured by the simulator, which does not emulate ST-LINK or the USB VCP bridge.
This is simulator evidence, not hardware confirmation.
