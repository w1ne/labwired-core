# NUCLEO-L432KC

ST's NUCLEO-L432KC (MB1180) is a Nucleo-32 board built around the STM32L432KC,
an 80 MHz Cortex-M4 MCU with 256 KB flash and 64 KB SRAM. It is a small,
low-power Nucleo option with Arduino Nano headers.

## Demand rank

STM32L432KC belongs to measured MCU demand rank #4, “other STM32” (13% of
respondents). Nucleo/Discovery kits are kit rank #4 at 23%. ST currently lists
the board as Active and In Stock; see the dated [vendor check](../../examples/nucleo-l432kc/vendor-status.md).

For the meaning of support levels, see the [target support rubric](../target_support_rubric.md).
The firmware exercise entry and evidence are listed in the [firmware-exercise matrix](FIRMWARE_EXERCISE_MATRIX.md).

## Status at a glance

| Aspect | Status |
| --- | --- |
| Exact part | STM32L432KCU6 on NUCLEO-L432KC |
| Model tier | Local deterministic UART and GPIO smoke; no silicon diff |
| Firmware | [Committed smoke source](../../crates/firmware-nucleo-l432kc-smoke/src/main.rs) |
| System | [nucleo-l432kc.yaml](../../configs/systems/nucleo-l432kc.yaml) |
| Evidence | [Committed smoke logs](../../examples/nucleo-l432kc/evidence/) |

## Flash and firmware artifact

The exact STM32L432KCU6 has 256 KB flash and 64 KB SRAM. The smoke firmware is
built for Cortex-M4 with the `thumbv7em-none-eabi` target. Its configured stop
reason is the 200,000-step limit; the committed result records each assertion.

## Board pins

| Signal | MCU pin | Modeled behavior |
| --- | --- | --- |
| ST-LINK VCP TX | PA2 / USART2_TX | AF7 transmit path and console output |
| ST-LINK VCP RX | PA15 / USART2_RX | Board route documented; receive traffic is not exercised |
| User LED LD3 / Nano D13 | PB3 | Active-high GPIO output |

The model follows the default UM1956 bridge setup. USB enumeration, ST-LINK
firmware, analog input, external clocks and peripherals outside RCC/GPIO/USART2
are not part of this profile.

## Support matrix

| Area | Status | Scope |
| --- | --- | --- |
| Cortex-M4 boot | ✅ | Smoke firmware boots from the configured flash map |
| RCC | ◐ | L4 register layout and GPIO/USART2 clock enables used by the smoke |
| GPIOA / GPIOB | ✅ | PA2 USART2_TX pad edges and PB3 LD3 edges are asserted |
| USART2 transmit | ✅ | Marker reaches the host console and TX edges route to PA2 |
| USART2 receive | ⚠️ | Board RX wiring is described; no receive firmware check is recorded |
| Other on-chip peripherals | ⚠️ | Not declared by this focused profile |

## What this catches versus a bench

The local deterministic smoke catches reset-vector loading, the selected L4
RCC clock gates, USART2 transmit output and AF7 routing on PA2, and active-high
PB3 LED transitions. It is simulator evidence only; it does not establish
silicon parity, electrical timing, ST-LINK USB behavior or physical board
acceptance.

## Copy-paste example

From the repository root, build and run the tested firmware:

    cargo build --release -p firmware-nucleo-l432kc-smoke --target thumbv7em-none-eabi
    cargo run --release -p labwired-cli -- test --script examples/nucleo-l432kc/io-smoke.yaml

The smoke checks the USART2 output marker, at least eight PA2 TX edges, and
at least eight PB3 LED edges.
The full tested firmware is [src/main.rs](../../crates/firmware-nucleo-l432kc-smoke/src/main.rs).

The hosted Playground/MCP catalog does not currently list NUCLEO-L432KC, so
this page makes no hosted verification claim.

## Related systems and examples

- [Board system descriptor](../../configs/systems/nucleo-l432kc.yaml)
- [Chip descriptor](../../configs/chips/stm32l432kc.yaml)
- [Example README and source references](../../examples/nucleo-l432kc/README.md)
- [Committed smoke firmware](../../crates/firmware-nucleo-l432kc-smoke/src/main.rs)
