# BlackPill F401CC Onboarding Validation

## Build Firmware
```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
```

## Run Smoke Simulation
```bash
cargo run -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-f401-demo \
  --system examples/blackpill-f401cc/system.yaml \
  --max-steps 100
```

Expected: "OK" printed to stdout, SP initialized, PC at Reset handler.

## Run CI Test Script
```bash
cargo build --release -p labwired-cli
./target/release/labwired test \
  --script examples/blackpill-f401cc/uart-smoke.yaml \
  --output-dir out/blackpill-f401cc-smoke \
  --no-uart-stdout
```

Expected: PASS with uart_contains "OK" assertion satisfied.
