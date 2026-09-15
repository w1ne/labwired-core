# Arduino Nano 33 IoT Validation Runbook

Run all commands from `core/`.

## 1) Optional: ensure target installed

```bash
rustup target add thumbv6m-none-eabi
```

## 2) Build smoke firmware

```bash
cargo build -p firmware-atsamd21-demo --release --target thumbv6m-none-eabi
```

## 3) Run deterministic UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nano-33-iot/uart-smoke.yaml \
  --output-dir out/nano-33-iot/uart-smoke \
  --no-uart-stdout
```

Pass criteria:
1. exit code is `0`
2. UART contains `OK`

## 4) Run firmware survival gate

```bash
cargo test -p labwired-core --test firmware_survival test_atsamd21_nano33_smoke_survival -- --nocapture
```

Pass criteria:
1. test passes
2. UART sink contains `OK` (fixture: `tests/fixtures/atsamd21-nano33-smoke.elf`)

## 5) Run direct simulation for PC/SP evidence

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv6m-none-eabi/release/firmware-atsamd21-demo \
  --system configs/systems/nano-33-iot.yaml \
  --max-steps 32 \
  --json
```

## 6) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv6m-none-eabi/release/firmware-atsamd21-demo \
  --system configs/systems/nano-33-iot.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nano-33-iot
```

Pass criteria:
1. script exits `0`
2. audit report exists at `out/unsupported-audit/nano-33-iot/report.md`
