# Validation — STM32F103C8T6 Blue Pill + BME280

The onboarding smoke builds register-level Cortex-M3 firmware, runs it on the
narrow Blue Pill profile, reads the attached BME280 ID register (`0xD0` →
`0x60`) over I2C1, and checks that the onboard PC13 LED changes state.

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103c8-bluepill-bme280 \
  --crate firmware-stm32f103c8-bluepill-bme280-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103c8-bluepill-bme280/io-smoke.yaml \
  --system configs/systems/stm32f103c8-bluepill-bme280.yaml \
  --out-dir examples/stm32f103c8-bluepill-bme280/evidence
```

The committed `evidence/` folder contains the build log, run log, UART output,
structured result, snapshot, and onboarding metrics. This is deterministic
digital-twin evidence; no physical-board capture is included.

The committed run passed all 3 checks in 200,000 steps on 2026-11-10. Rebuild
the concise timing scoreboard after rerunning the smoke:

```sh
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/stm32f103c8-bluepill-bme280/evidence \
  --markdown-out examples/stm32f103c8-bluepill-bme280/evidence/onboarding-scoreboard.md \
  --json-out examples/stm32f103c8-bluepill-bme280/evidence/onboarding-scoreboard.json
```

The optional hosted-model verification is recorded separately in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md); it
uses a hosted STM32F103 board and validates the BME280 model/address path, not
this exact repository firmware binary or Blue Pill profile.
