# ST NUCLEO-F103RB

The NUCLEO-F103RB is an ST Nucleo-64 development board with an STM32F103RB
Cortex-M3 in LQFP64; boards commonly ship with STM32F103RBT6. The LabWired
system describes the carrier board and its on-board LED, button, and
ST-LINK virtual COM port.

## Status at a glance

| Area | Status |
|---|---|
| Exact chip geometry | Declared: 128 KB flash, 20 KB SRAM, LQFP64 |
| Board wiring | Declared from ST UM1724 |
| GPIO + USART2 smoke | Simulator-verified: UART `OK`, 10 PA5 edges, 200,000 steps |
| Hardware comparison | Not recorded |
| External devices | None in the base board system |

See the [F103RB example](../../examples/nucleo-f103rb/README.md),
[validation runbook](../../examples/nucleo-f103rb/VALIDATION.md), and
[known limitations](../../examples/nucleo-f103rb/KNOWN_LIMITATIONS.md).

## Chip and board files

- Exact chip descriptor: [`stm32f103rb.yaml`](../../configs/chips/stm32f103rb.yaml)
- Carrier system: [`nucleo-f103rb.yaml`](../../configs/systems/nucleo-f103rb.yaml)
- Smoke firmware: `crates/firmware-nucleo-f103rb-smoke/`
- Runnable example: `examples/nucleo-f103rb/`

The model shares the family reference's medium-density F103 peripheral map.
Its exact geometry identifies the STM32F103RB, while behavior beyond the
documented smoke remains unvalidated for this specific part.

## Board pins

| Function | Pin | Polarity / route |
|---|---|---|
| LD2 user LED | PA5 | Active high |
| B1 user button | PC13 | Active low |
| USART2 TX/RX | PA2 / PA3 | Default ST-LINK/V2-1 VCP route |
| SWDIO / SWCLK | PA13 / PA14 | ST-LINK debug |
| Morpho GPIO | PD0 / PD1 | Shared with OSC_IN / OSC_OUT |
| Morpho GPIO | PD2 | General-purpose I/O |

The pin mapping and active LED level follow ST UM1724. The VCP connection is
controlled by solder bridges SB13/SB14.

## How to run

Build `firmware-nucleo-f103rb-smoke` for `thumbv7m-none-eabi`, then run the
example's [`io-smoke.yaml`](../../examples/nucleo-f103rb/io-smoke.yaml). The
recorded simulator run captured `OK` on UART and 10 PA5 edges. The exact chip
ID is `stm32f103rb`; the carrier board ID is `nucleo-f103rb`.

## Support boundary

The runnable scope is GPIOA/PA5 and USART2/PA2-PA3. B1 is declared as board I/O
but is not exercised. External bus devices, clock-tree behavior, timers, DMA,
ADC, SPI, I2C, USB, CAN, and other unexercised peripherals are outside the
current claim. See the [validation boundary](../../examples/nucleo-f103rb/KNOWN_LIMITATIONS.md).
