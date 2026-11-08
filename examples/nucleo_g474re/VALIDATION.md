# Validation

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g474re \
  --crate firmware-nucleo_g474re-demo \
  --firmware-build-dir examples/nucleo_g474re/board_firmware \
  --target thumbv7em-none-eabi \
  --script examples/nucleo_g474re/uart-smoke.yaml \
  --system configs/systems/nucleo_g474re.yaml \
  --out-dir examples/nucleo_g474re/evidence
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nucleo_g474re/evidence \
  --markdown-out examples/nucleo_g474re/evidence/onboarding-scoreboard.md \
  --json-out examples/nucleo_g474re/evidence/onboarding-scoreboard.json
```

Expected evidence:

- The firmware boots from the declared flash map and prints its LPUART1 marker.
- GPIOA clock enable and LD2/PA5 output produce at least eight edges.
- `evidence/logs/run_smoke.log` records a passing CLI smoke.

The smoke does not validate USART1, the other declared peripherals, the UART
pad waveform, or physical silicon. The broader chip descriptor's current
coverage is summarized in [`docs/boards/stm32g474re.md`](../../docs/boards/stm32g474re.md).

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo_g474re/board_firmware/target/thumbv7em-none-eabi/release/firmware-nucleo_g474re-demo \
  --system configs/systems/nucleo_g474re.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo_g474re
```
