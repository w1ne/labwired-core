# NUCLEO-L432KC

This profile is for ST's NUCLEO-L432KC (MB1180) with an STM32L432KCU6. The
MCU is in demand rank #4, “other STM32” (13%); STM32 Nucleo/Discovery kits are
also kit rank #4 (23%). The profile covers the board's default USART2 VCP
connection and active-high user LED LD3. It does not claim ST-LINK USB/debug,
other MCU peripherals, or a silicon comparison.

## Pin map

| Board signal | STM32 pin | Model path |
| --- | --- | --- |
| ST-LINK VCP TX | PA2 / USART2_TX | USART2, AF7 |
| ST-LINK VCP RX | PA15 / USART2_RX | USART2 receive pin declared; RX traffic not exercised |
| User LED LD3 / Nano D13 | PB3 | GPIOB output, active high |

UM1956 documents SB2/SB3 as the default VCP bridge settings and SB15 as the
default connection from LD3 to PB3. The board is modeled at its reset MSI clock
of 4 MHz. The smoke builds a Cortex-M4 image and asserts the USART2 marker,
USART2_TX edges on PA2, and LD3 edges on PB3.

## Copy-paste smoke

From the repository root, build and run the committed firmware:

    cargo build --release -p firmware-nucleo-l432kc-smoke --target thumbv7em-none-eabi
    cargo run --release -p labwired-cli -- test --script examples/nucleo-l432kc/io-smoke.yaml

The tested firmware source is [src/main.rs](../../crates/firmware-nucleo-l432kc-smoke/src/main.rs).
The deterministic local smoke logs are committed under [evidence/](evidence/).

## Vendor status

The live check and source references are in [vendor-status.md](vendor-status.md)
and [REQUIRED_DOCS.md](REQUIRED_DOCS.md).
