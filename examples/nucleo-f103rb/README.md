# NUCLEO-F103RB onboarding example

Run the commands from the Core repository root. This example targets the real
NUCLEO-F103RB carrier board with its STM32F103RBT6, not the generic STM32F103C8
model.

## Smoke scope

The bare-metal firmware enables GPIOA and USART2, sends `OK` over the default
ST-LINK virtual COM port, and toggles LD2. It uses the F1 GPIO CRL and USART
register layout and the reset HSI clock. No external parts are required.

| Board signal | MCU pin | Behavior |
|---|---|---|
| LD2 green user LED | PA5 | Active high |
| B1 user button | PC13 | Active low |
| ST-LINK virtual COM port TX/RX | PA2 / PA3 | USART2 |
| ST-LINK debug | PA13 / PA14 | SWDIO / SWCLK |
| Morpho GPIO | PD0 / PD1 | Shared with OSC_IN / OSC_OUT |
| Morpho GPIO | PD2 | General-purpose I/O |

UM1724 identifies NUCLEO-F103RB in its MB1136 board family. Its default
USART2-to-ST-LINK connection is selected through SB13/SB14. LD2 is connected
to PA5 and lights when the I/O is high; B1 is connected to PC13.

## Build and run

```sh
rustup target add thumbv7m-none-eabi
cargo build -p firmware-nucleo-f103rb-smoke --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/firmware-nucleo-f103rb-smoke \
  --system configs/systems/nucleo-f103rb.yaml \
  --max-steps 200000
```

Expected UART output begins with:

```text
OK
```

The runnable assertion scripts are `io-smoke.yaml` (UART plus four LD2 GPIO
edges) and `uart-smoke.yaml` (UART output).

## Source and validation boundary

The board wiring follows ST UM1724 and the chip geometry follows ST DS5319.
The model shares the STM32F103 medium-density peripheral definitions with the
family reference descriptor. The smoke is simulation-side evidence only; see
[`VALIDATION.md`](VALIDATION.md) and
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md) before relying on unexercised
peripherals.
