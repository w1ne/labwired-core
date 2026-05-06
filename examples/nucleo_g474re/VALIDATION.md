# Validation

Run from `core/`:

```bash
cargo build --manifest-path examples/nucleo_g474re/board_firmware/Cargo.toml --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo_g474re/uart-smoke.yaml \
  --output-dir out/nucleo_g474re/uart-smoke \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo_g474re/board_firmware/target/thumbv7em-none-eabi/release/firmware-nucleo_g474re-demo \
  --system configs/systems/nucleo_g474re.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo_g474re
```

Expected evidence:

- PC/SP initialize correctly
- UART smoke output is deterministic
- LED/button paths are mapped and exercised
