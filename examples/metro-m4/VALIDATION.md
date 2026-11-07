# Adafruit Metro M4 Validation Runbook

Run all commands from `core/`.

## 1) Optional: ensure target installed

```bash
rustup target add thumbv7em-none-eabi
```

## 2) Build smoke firmware

```bash
cargo build -p firmware-atsamd51-demo --release --target thumbv7em-none-eabi
```

## 3) Run the committed onboarding smoke

```bash
scripts/onboarding_smoke.sh \
  --target-id metro-m4 \
  --crate firmware-atsamd51-demo \
  --target thumbv7em-none-eabi \
  --script examples/metro-m4/onboarding-smoke.yaml \
  --system configs/systems/metro-m4.yaml \
  --out-dir examples/metro-m4/evidence
```

Pass criteria:
1. exit code is `0`
2. UART contains `METRO_M4_OK`
3. `porta:16` records at least one GPIO edge

## 4) Run firmware survival gate

```bash
cargo test -p labwired-core --test firmware_survival test_atsamd51_metro_m4_smoke_survival -- --nocapture
```

Pass criteria:
1. test passes
2. UART sink contains `OK` (fixture: `tests/fixtures/atsamd51-metro-m4-smoke.elf`)

## 5) Run direct simulation for PC/SP evidence

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-atsamd51-demo \
  --system configs/systems/metro-m4.yaml \
  --max-steps 32 \
  --json
```

## 6) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-atsamd51-demo \
  --system configs/systems/metro-m4.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/metro-m4
```

Pass criteria:
1. script exits `0`
2. audit report exists at `out/unsupported-audit/metro-m4/report.md`
