# NUCLEO-G0B1RE quick start

## Demand rank

Other STM32 MCUs are demand rank **#4** at 13% reported use, and the Nucleo/Discovery kit category appears in **23%** of main projects. These are survey categories, not a G0B1RE-specific percentage. ST lists this exact board and its STM32G0B1RE as Active; the board was in stock at ST's store when checked. See [vendor-status.md](vendor-status.md).

## Copy-paste smoke

From the repository root, build the Cortex-M0+ firmware and simulate the exact NUCLEO-G0B1RE profile:

```bash
cargo build -p firmware-nucleo-g0b1re-smoke --release --target thumbv6m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-g0b1re/io-smoke.yaml \
  --output-dir out/nucleo-g0b1re/io-smoke --no-uart-stdout
```

The smoke expects `G0B1_USART2_OK LD4=PA5` from USART2 and at least eight output transitions on PA5, the active-high onboard LD4. The board profile keeps the MCU at its 16 MHz reset clock. UART bytes are observed at the USART2 register model; the ST-LINK USB bridge and PA2/PA3 pad waveform are outside this smoke. It does not claim full-chip coverage or physical-board confirmation.

The committed build and simulation logs are in [evidence/](evidence/).
