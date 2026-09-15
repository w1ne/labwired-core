# Arduino Uno R4 Minima Validation Runbook

Run all commands from `core/`.

## 1) Optional: ensure target installed

```bash
rustup target add thumbv7em-none-eabi
```

## 2) Build smoke firmware

```bash
cargo build -p firmware-ra4m1-demo --release --target thumbv7em-none-eabi
```

## 3) Run deterministic UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-r4-minima/uart-smoke.yaml \
  --output-dir out/arduino-uno-r4-minima/uart-smoke \
  --no-uart-stdout
```

Pass criteria:
1. exit code is `0`
2. UART contains `OK`

## 4) Run firmware survival gate

```bash
cargo test -p labwired-core --test firmware_survival test_ra4m1_uno_r4_smoke_survival -- --nocapture
```

Pass criteria:
1. test passes
2. UART sink contains `OK` (fixture: `tests/fixtures/ra4m1-uno-r4-smoke.elf`)

## 5) Run direct simulation for PC/SP evidence

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-ra4m1-demo \
  --system configs/systems/arduino-uno-r4-minima.yaml \
  --max-steps 32 \
  --json
```

## 6) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-ra4m1-demo \
  --system configs/systems/arduino-uno-r4-minima.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/arduino-uno-r4-minima
```

Pass criteria:
1. script exits `0`
2. audit report exists at `out/unsupported-audit/arduino-uno-r4-minima/report.md`
