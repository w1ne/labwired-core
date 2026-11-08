# NUCLEO-WB55RG

ST's NUCLEO-WB55RG (MB1355C) carries the STM32WB55RG dual-core wireless MCU
with 1 MiB flash and 256 KiB SRAM. This system profile models the Cortex-M4
application core, the three user LEDs, the three user buttons, and USART1 on
the default ST-LINK VCP1 pins.

| Board signal | MCU pin | Polarity / route |
| --- | --- | --- |
| LED1 blue | PB5 | Active high |
| LED2 green | PB0 | Active high |
| LED3 red | PB1 | Active high |
| SW1 | PC4 | Active low; default solder bridge |
| SW2 | PD0 | Active low |
| SW3 | PD1 | Active low |
| USART1 TX/RX | PB6/PB7, AF7 | ST-LINK VCP1, default jumpers |

The onboarding firmware exercises all three LED GPIOs and sends ordered
heartbeats through USART1. The local simulator captures the target UART
register output; it does not model ST-LINK firmware, USB enumeration, or the
physical VCP bridge. LPUART1 on PA2/PA3 is routed to Arduino D1/D0 and is not
used as the default VCP console.

The Cortex-M0+ radio core, RF transceiver, BLE, Thread, Zigbee, and 802.15.4
stacks are omitted. They are not replaced with zero-answering stubs. The
smoke also does not exercise the user buttons or other MCU peripherals.

The simulator-derived smoke and committed evidence are in
[`examples/nucleo-wb55rg/`](../../examples/nucleo-wb55rg/). ST's live board and
MCU lifecycle/orderability check is recorded in
[`vendor-status.md`](../../examples/nucleo-wb55rg/evidence/vendor-status.md).

## Sources

- ST [UM2819, STM32WB Nucleo-64 board (MB1355)](https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf), §§7.6.3, 7.7, 7.8.
- Zephyr [NUCLEO-WB55RG board DTS](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/st/nucleo_wb55rg/nucleo_wb55rg.dts), GPIO assignments and polarities.
- ST [NUCLEO-WB55RG live product status](https://www.st.com/en/evaluation-tools/nucleo-wb55rg.html).
