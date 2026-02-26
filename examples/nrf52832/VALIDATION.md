# nRF52832 Onboarding Validation

## Build Firmware
```bash
cargo build -p firmware-nrf52832-demo --release --target thumbv7em-none-eabi
```

## Run Smoke Simulation
```bash
cargo run -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-nrf52832-demo \
  --system examples/nrf52832/system.yaml \
  --max-steps 100
```

Expected: "OK" printed to stdout, SP=0x20010000, PC initializes to Reset handler.

## Run CI Test Script
```bash
cargo build --release -p labwired-cli
./target/release/labwired test \
  --script examples/nrf52832/uart-smoke.yaml \
  --output-dir out/nrf52832-smoke \
  --no-uart-stdout
```

Expected: PASS with uart_contains "OK" assertion satisfied.

## Unsupported Instruction Audit
```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-nrf52832-demo \
  --system configs/systems/nrf52832-example.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nrf52832
```
