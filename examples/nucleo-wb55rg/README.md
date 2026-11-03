# NUCLEO-WB55RG (MB1355C)

This profile boots firmware on the STM32WB55RG application core (1 MiB flash,
256 KiB SRAM), blinks all
three on-board user LEDs, and sends its heartbeat through USART1 on the
default ST-LINK Virtual COM Port pins. The board system is
[`configs/systems/mb1355c.yaml`](../../configs/systems/mb1355c.yaml); its
chip model is [`configs/chips/stm32wb55.yaml`](../../configs/chips/stm32wb55.yaml).
The board-level scope and pin map are also recorded in
[`docs/boards/nucleo-wb55rg.md`](../../docs/boards/nucleo-wb55rg.md).

The profile exposes these board connections:

| Signal | STM32WB55 pin | Board connection | Polarity |
| --- | --- | --- | --- |
| LED1 blue | PB5 | User LED | Active high |
| LED2 green | PB0 | User LED | Active high |
| LED3 red | PB1 | User LED | Active high |
| SW1 | PC4 | User button, default solder bridge | Active low |
| SW2 | PD0 | User button | Active low |
| SW3 | PD1 | User button | Active low |
| USART1 TX/RX | PB6/PB7, AF7 | ST-LINK VCP1, default jumpers | — |

The smoke asserts the `NUCLEO-WB55RG ready` UART marker, ordered LED
heartbeats, and GPIO edges on PB5, PB0, and PB1. It models the target UART
registers; it does not simulate ST-LINK firmware, USB enumeration, or the
physical VCP bridge.

Run the committed smoke with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-wb55rg \
  --crate firmware-nucleo-wb55rg-demo \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-wb55rg/io-smoke.yaml \
  --system configs/systems/mb1355c.yaml \
  --out-dir examples/nucleo-wb55rg/evidence
```

Build logs, simulator output, scoreboard, metrics, and vendor status are
committed under [`evidence/`](evidence/). See
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md) for the unmodeled radio and
second core.

## Sources

- ST [NUCLEO-WB55RG product page](https://www.st.com/en/evaluation-tools/nucleo-wb55rg.html)
- ST [UM2819, STM32WB Nucleo-64 board (MB1355)](https://www.st.com/resource/en/user_manual/um2819-stm32wb-nucleo64-board-mb1355-stmicroelectronics.pdf), §§7.6.3, 7.7, 7.8
- Zephyr [NUCLEO-WB55RG board DTS](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/st/nucleo_wb55rg/nucleo_wb55rg.dts), LED/button GPIO assignments and polarities
- Zephyr [NUCLEO-WB55RG board documentation](https://docs.zephyrproject.org/latest/boards/st/nucleo_wb55rg/doc/nucleo_wb55rg.html), default peripheral pin mapping
