# Validation

Run from `core/`:

```bash
cargo build --manifest-path examples/nucleo_wba52cg/board_firmware/Cargo.toml --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo_wba52cg/uart-smoke.yaml \
  --output-dir out/nucleo_wba52cg/uart-smoke \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo_wba52cg/board_firmware/target/thumbv8m.main-none-eabi/release/firmware-nucleo_wba52cg-demo \
  --system configs/systems/nucleo_wba52cg.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo_wba52cg
```

Expected evidence:

- PC/SP initialize correctly
- UART smoke output is deterministic
- LED/button paths are mapped and exercised
