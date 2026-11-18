# NUCLEO-U5A5ZJ-Q Example (STM32U5A5ZJ)

This is the crypto-capable U5A5 variant on the MB1549 Nucleo-144 board. The
board shares its LED, button and VCP wiring with NUCLEO-U575ZI-Q; the chip
profile keeps the U5A5's 4 MiB flash, additional SRAM banks, SAES and PKA.

Run both board smokes from the Core workspace root:

```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u5a5zj/io-smoke.yaml
examples/nucleo-u5a5zj/ci/build-saes-smoke.sh
cargo run -q -p labwired-cli -- test --script examples/nucleo-u5a5zj/saes-smoke.yaml
```

The basic smoke checks the USART1 VCP. The SAES smoke enables the SAES clock,
programs AES-128 ECB with the NIST SP 800-38A key and first plaintext block,
and prints `U5A5_SAES_AES128_ECB_OK` only when the model returns the expected
ciphertext. This adds firmware-driven coverage of the U5A5-only SAES peripheral.
It does not validate hardware timing, side-channel behavior, TrustZone key
handling, or physical silicon parity. The model is simulator-validated; no
physical U5A5 board test is claimed.

See [`vendor-status.md`](vendor-status.md) for the live ST product and store
availability receipt.
