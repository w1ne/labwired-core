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

Refresh the checked-in fixture after changing the firmware source:

```bash
examples/arduino-uno-r4-minima/ci/refresh-survival-fixture.sh
```

Then run the same board case with the feature-off graph used by PR CI:

```bash
cargo test -p labwired-core --no-default-features --test gates \
  firmware_survival::test_ra4m1_uno_r4_smoke_survival -- --nocapture
```

The default-feature control is:

```bash
cargo test -p labwired-core --test gates firmware_survival::test_ra4m1_uno_r4_smoke_survival -- --nocapture
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

## 7) Run the mixed-signal analog coverage test

The board's `analog_pins:` map (A0-A5 → P014/P000/P001/P002/P101/P100) is proven
by the generic NTC divider, Shockley diode load line and A4 divider in
`analog-ntc-diode-system.yaml`:

```bash
cargo test -p labwired-core --test analog cosim_ra4m1_analog -- --nocapture
```

Pass criteria:
1. both tests pass (`uno_r4_minima_*`)
2. `docs/analog_board_coverage.md` lists `arduino-uno-r4-minima` and
   `arduino-uno-r4-minima-bme280` under **Covered**

