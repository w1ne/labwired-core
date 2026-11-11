# STM32F103C8 Blue Pill reference profile

The STM32F103C8T6 is an active, purchasable 72 MHz Cortex-M3 part with 64 KiB
flash and 20 KiB SRAM. This profile covers a common Blue Pill reference layout
and is **board smoke scope**, not a full model of every Blue Pill copy.

## Demand rank

The STM32F103 family is demand rank **#3** at 17% in the JetBrains State of
Embedded 2025 survey. The Blue Pill is the best-known low-cost F103 board. The
ATmega328P ranks #2 but is NRND, so it is not a candidate for new onboarding.

## Board pins in this profile

| Blue Pill signal | MCU pin | Model behavior |
| --- | --- | --- |
| User LED | PC13, active low | GPIO output recorded by `board_io` |
| USART1 TX | PA9 | Captured by the board's debug UART path |
| USART1 RX | PA10 | Present in the UART pin mapping; RX firmware is not in this smoke |

The board profile uses the reset HSI clock at 8 MHz. UART uses an external
adapter connected to PA9/PA10; the Blue Pill has no integrated USB-to-UART
bridge.

## Copy-paste run

```bash
cargo build -p firmware-stm32f103-bluepill-smoke --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f103-bluepill/io-smoke.yaml \
  --output-dir out/stm32f103-bluepill/io-smoke --no-uart-stdout
```

## Limits and evidence

The profile does not claim USB device behavior, USB boot/programming, external
clock startup, reset/boot-jumper interactions, or cloned MCUs. It has no
zero-answering peripherals in its descriptor. The broader F103 descriptor
still contains `usb_dev` and `bkp` explicit stubs; they are not inherited here.

See [validation and evidence](../../examples/stm32f103-bluepill/VALIDATION.md)
and [live availability](../../examples/stm32f103-bluepill/vendor-status.md).
