# NUCLEO-U5A5ZJ-Q onboarding record

## Board and live status

ST's product page listed NUCLEO-U5A5ZJ-Q as **Active / in volume production**
on 2026-11-03. ST's eStore showed **In Stock**, **Add to Cart**, and a displayed
price of $38.96 for two units. The dated URLs and exact check are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board scope

The board system uses the STM32U5A5ZJT6Q chip profile in
[`configs/systems/nucleo-u5a5zj.yaml`](../../configs/systems/nucleo-u5a5zj.yaml).
The onboarding smoke exercises:

- USART1 VCP: PA9 TX / PA10 RX.
- LD1 green: PC7; LD2 blue: PB7; LD3 red: PG2. All three are active-high.
- MCU blocks used by this smoke: RCC, GPIOB/GPIOC/GPIOG, GPIOA and USART1.

The firmware prints `U5A5ZJ_OK` and toggles all three LED outputs. The
committed local simulator run and generated onboarding metrics are under
[`evidence/`](evidence/). No physical-board test is claimed. The optional B1
input and board USB are not exercised here. The U5A5 pin map uses the documented
U575 common GPIO/alternate-function subset rather than a complete U5A5 AF map;
crypto peripheral limitations are listed in
[`docs/boards/stm32u5a5zj.md`](../../docs/boards/stm32u5a5zj.md).
