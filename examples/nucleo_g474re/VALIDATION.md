# Validation

Run the board smoke from the repository root:

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

- The bare-metal image reaches the USART2 `OK` marker.
- TIM1 channel 1 drives PA8/AF6 and produces at least eight GPIO transitions.
- The structured result reports all assertions passing and stops at the
  configured step limit.

This smoke does not validate other G4 timer output mappings, the other
peripherals in the chip descriptor, or physical silicon. This change adds
only the PA8/TIM1_CH1 route; the existing G4 timer routes are outside this
smoke's assertions.
