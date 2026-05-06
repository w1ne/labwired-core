# Validation

Run from `core/`:

```bash
cargo build --manifest-path examples/mb1355c/board_firmware/Cargo.toml --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/mb1355c/uart-smoke.yaml \
  --output-dir out/mb1355c/uart-smoke \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/mb1355c/board_firmware/target/thumbv7em-none-eabi/release/firmware-mb1355c-demo \
  --system configs/systems/mb1355c.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/mb1355c
```

Expected evidence:

- PC/SP initialize correctly
- UART smoke output is deterministic
- LED/button paths are mapped and exercised
