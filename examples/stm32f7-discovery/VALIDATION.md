# STM32F7 Discovery Validation Runbook

Run all commands from `core/`.

## 1) Optional: ensure soft-float target installed

```bash
rustup target add thumbv7em-none-eabi
```

Do **not** use `thumbv7em-none-eabihf` for this smoke.

## 2) Build smoke firmware

```bash
cargo build -p firmware-stm32f746-demo --release --target thumbv7em-none-eabi
```

## 3) Run deterministic UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f7-discovery/uart-smoke.yaml \
  --output-dir out/stm32f7-discovery/uart-smoke \
  --no-uart-stdout
```

Pass criteria:
1. exit code is `0`
2. UART contains `OK`

## 4) Run firmware survival gate

```bash
cargo test -p labwired-core --test firmware_survival test_stm32f746_discovery_smoke_survival -- --nocapture
```

Pass criteria:
1. test passes
2. UART sink contains `OK` (fixture: `tests/fixtures/stm32f746-discovery-smoke.elf`)

## 5) Run config-build gate

```bash
cargo test -p labwired-core --test stm32f746_config -- --nocapture
```

## 6) Run direct simulation for PC/SP evidence

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-stm32f746-demo \
  --system configs/systems/stm32f7-discovery.yaml \
  --max-steps 32 \
  --json
```
