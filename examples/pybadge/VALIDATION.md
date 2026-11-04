# Adafruit PyBadge validation record

## Reproduce the onboarding smoke

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id adafruit-pybadge \
  --crate firmware-atsamd51-pybadge-demo \
  --target thumbv7em-none-eabi \
  --script examples/pybadge/onboarding-smoke.yaml \
  --system examples/pybadge/system.yaml \
  --out-dir examples/pybadge/evidence
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/pybadge/evidence \
  --markdown-out examples/pybadge/evidence/onboarding-scoreboard.md \
  --json-out examples/pybadge/evidence/onboarding-scoreboard.json
```

The smoke rebuilds the bare-metal image linked at `0x4000`, behind the
resident 16 KiB UF2 bootloader. Its acceptance checks require `OK` on SERCOM1,
at least four GPIO edges on PA23 (D13), and the expected max-step stop. The
committed logs and result artifacts are in `evidence/`.

## Scope

The example system maps the modeled Feather UART on SERCOM1/PA16-PA17 and the
active-high D13 LED on PORTA/PA23. The smoke verifies those two paths. The
ST7735 display is not attached; the board's buttons and NeoPixels are not
exercised by this smoke. USB and QSPI remain zero-answering stub windows in
the chip descriptor. No silicon capture is available.

The live board and MCU availability check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
