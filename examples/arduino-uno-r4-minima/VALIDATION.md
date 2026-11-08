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

## 3) Run the onboarding UART and GPIO smoke

```bash
scripts/onboarding_smoke.sh \
  --target-id arduino-uno-r4-minima \
  --crate firmware-ra4m1-demo \
  --target thumbv7em-none-eabi \
  --script examples/arduino-uno-r4-minima/io-smoke.yaml \
  --system configs/systems/arduino-uno-r4-minima.yaml \
  --out-dir examples/arduino-uno-r4-minima/evidence
```

Pass criteria:
1. onboarding metrics report `pass`
2. SCI2 output contains `OK`
3. PORT1/P111 records at least one GPIO edge

## 4) Run the focused UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-r4-minima/uart-smoke.yaml \
  --output-dir out/arduino-uno-r4-minima/uart-smoke \
  --no-uart-stdout
```

Pass criteria:
1. exit code is `0`
2. UART contains `OK`

## 5) Run firmware survival gate

```bash
cargo test -p labwired-core --test firmware_survival test_ra4m1_uno_r4_smoke_survival -- --nocapture
```

Pass criteria:
1. test passes
2. UART sink contains `OK` (fixture: `tests/fixtures/ra4m1-uno-r4-smoke.elf`)

## 6) Run direct simulation for PC/SP evidence

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-ra4m1-demo \
  --system configs/systems/arduino-uno-r4-minima.yaml \
  --max-steps 32 \
  --json
```

## 7) Run unsupported-instruction audit

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
