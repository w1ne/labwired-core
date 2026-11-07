# NUCLEO-U5A5ZJ-Q Validation Runbook

Run commands from the Core repository root. The target is **sim-validated**:
the descriptors and peripheral behavior are simulator models based on ST's
reference material. No physical U5A5 capture or acceptance is claimed.

## Onboarding board I/O smoke

The dedicated onboarding firmware exercises USART1 and toggles all three
active-high user LEDs: LD1/PC7, LD2/PB7 and LD3/PG2. Build and capture the
committed evidence with:

```bash
./scripts/onboarding_smoke.sh \
  --target-id nucleo-u5a5zj-q \
  --crate firmware-stm32u575-demo \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-u5a5zj/onboarding-smoke.yaml \
  --system configs/systems/nucleo-u5a5zj.yaml \
  --out-dir examples/nucleo-u5a5zj/evidence
```

The committed local simulator result must report five passing assertions:
USART1 contains `U5A5ZJ_OK`, each LED pin has at least eight edges, and the run
reaches the configured step limit. `evidence/vendor-status.md` records the
live ST status and stock check. The hosted verifier attempt is separately
recorded in `evidence/hosted-verification.json`; its Zephyr build environment
currently stops before compilation because it has Python 3.11.2 instead of
the required Python 3.12 or later.

## Existing UART-only example smoke

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
