# Validation

Run from `core/`:

```bash
cargo build --manifest-path examples/nucleo_g474re/board_firmware/Cargo.toml --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo_g474re/tim1-pwm-smoke.yaml \
  --output-dir out/nucleo_g474re/tim1-pwm-smoke \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo_g474re/board_firmware/target/thumbv7em-none-eabi/release/firmware-nucleo_g474re-demo \
  --system configs/systems/nucleo_g474re.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo_g474re
```

Expected evidence:

- PC/SP initialize correctly
- USART2 prints `OK`
- TIM1_CH1 on PA8, routed through GPIOA AF6, produces at least four observed
  transitions at the configured GPIO pad

The smoke proves the single-ended main output only. Complementary TIM1_CHxN
outputs, dead-time insertion, break inputs, center-aligned counting and the
Nucleo's PLL clock setup remain outside this descriptor's exercised scope.

The repository CLI passed the UART and PA8 edge assertions. The separate
hosted `labwired_verify` check returned `failed`: its G474 run captured zero
PA8 transitions. Hosted verification therefore remains unconfirmed; this
result is recorded separately from the local CLI evidence below. No physical
silicon comparison was run.

Committed local evidence: `evidence/logs/` and `evidence/simulation-output/`.
