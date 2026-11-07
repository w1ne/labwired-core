# Validation — STM32F103C8T6 Blue Pill

**Simulator smoke passed on 2026-11-05.** The evidence covers the active-low
PC13 GPIO LED path only; it is not a physical-board test.

Reproduce from the repository root with:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103c8-bluepill \
  --crate firmware-stm32f103c8-bluepill-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103c8-bluepill/io-smoke.yaml \
  --system configs/systems/stm32f103c8-bluepill.yaml \
  --out-dir examples/stm32f103c8-bluepill/evidence
```

Recorded result:

- Build and smoke status: **pass**.
- Firmware SHA-256: `6acd3703f78b380feaf076f92cdb49c20121e3d59b37ac2493f31cb7c91423e6`.
- Smoke assertions: **2/2**; GPIOC pin 13 recorded **11 transitions**, and
  the run stopped at the requested 200,000 steps.
- The loaded chip descriptor declares RCC and GPIOC only; the profile's flash
  and SRAM sizes match the STM32F103C8T6.

The committed `evidence/` directory contains the build logs, simulation
transcript, result, snapshot, metrics, and scoreboard. No physical GPIO or LED
capture has been made. The simulator assumes an ideal 3.3 V supply rather than
simulating the Blue Pill regulator and power-on behavior.
