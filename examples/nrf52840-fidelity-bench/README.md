# nRF52840 fidelity cases

Four images from `firmware/main.c`. A case passes when its marker reaches the UART.
LabWired must match the silicon column.

| case | silicon | marker | why |
| --- | --- | --- | --- |
| `nrf-control` | PASS | `BENCH_NRF_OK` | legacy UART0 TXD prints the marker |
| `uarttime` | PASS | `BENCH_UART_TIME` | TXDRDY is still clear 64 nops after TXD; silicon raises it after the stop bit |
| `rtcclock` | PASS | `BENCH_RTC_OK` | RTC0 counter is unchanged after 32 CPU nops; it runs from 32.768 kHz |
| `flashbound` | PASS | `BENCH_FLASH_OK` | ERASEPAGE past the 1 MB flash leaves the last real page alone |

Needs `arm-none-eabi-gcc`.
