# NUCLEO-U5A5ZJ-Q Validation Runbook

Run commands from the Core repository root. The target is **sim-validated**:
the descriptors and peripheral behavior are simulator models based on ST's
reference material. No physical U5A5 capture or acceptance is claimed.

## Board I/O smoke

The shared U575 demo firmware is used for its USART1 smoke path. It verifies
that the Nucleo system boots and captures the `OK` UART output; it is not a
U5A5 silicon comparison.

```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u5a5zj/io-smoke.yaml
```

Pass criteria: exit code 0 and UART output `OK`.

## HASH, SAES and PKA register behavior

The register-path oracle drives the modeled peripherals through `SystemBus`.
It covers SHA-256, AES-128 ECB, modular exponentiation, and negative PKA
arithmetic subtraction with sign extension through 4160-bit operands. It
rejects widths above 4160 bits.

```bash
cargo test -p labwired-core --test stm32u5_exec_oracle -- --nocapture
cargo test -p labwired-core --test stm32u5_reset_conformance -- --nocapture
```

Pass criteria: the oracle tests and memory/reset conformance test pass. This
does not establish cryptographic side-channel properties, hardware timing,
complete PKA mode coverage, or silicon parity.

## Validation record

- 2026-10-26: simulator register-path and reset conformance checks are recorded
  in `validation/manifest.yaml` and gated by the Core CI validation generator.
  Physical board acceptance remains pending.
