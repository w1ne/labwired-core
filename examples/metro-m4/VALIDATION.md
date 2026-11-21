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

## 3) Run deterministic UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/metro-m4/uart-smoke.yaml \
  --output-dir out/metro-m4/uart-smoke \
  --no-uart-stdout
```

Pass criteria:
1. exit code is `0`
2. UART contains `OK`

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

## 7) Run mixed-signal ADC0 proof

```bash
cargo test -p labwired-core --test cosim_metro_m4_analog -- --nocapture
```

Pass criteria:
1. `metro_m4_ntc_and_diode_netlists_reach_adc0_a0_and_a4` passes (NTC on A0, diode on A4).
2. `metro_m4_mosfet_junction_density_reaches_adc0_alongside_ntc_and_diode` passes:
   `analog-mosfet-junction-density.yaml` drives three NMOS bulk junctions on
   A1/A2/A3 (AIN5/AIN6/AIN4). The solved voltages match ngspice-45 `.op` within
   0.1 mV at 3.3 V and an independent load line at 3.3 V and 2.5 V, and each
   ADC0 result equals the code for its solved voltage.
