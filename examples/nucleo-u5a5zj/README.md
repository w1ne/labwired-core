# NUCLEO-U5A5ZJ-Q Example (STM32U5A5ZJ)

This is the crypto-capable U5A5 variant on the MB1549 Nucleo-144 board. The
board shares its LED, button and VCP wiring with NUCLEO-U575ZI-Q; the chip
profile keeps the U5A5's 4 MiB flash, additional SRAM banks, SAES and PKA.

Run the basic board smoke from the Core workspace root:

```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u5a5zj/io-smoke.yaml
```

The shared demo firmware is used only for its USART1 `OK` smoke. Crypto
register-level operations are covered by `crates/core/tests/stm32u5_exec_oracle.rs`.
The model is simulator-validated; no physical U5A5 board test is claimed.
