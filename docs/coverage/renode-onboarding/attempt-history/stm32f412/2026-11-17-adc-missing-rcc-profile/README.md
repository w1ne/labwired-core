# STM32F412 ADC1 failure investigation

## Frozen attempt

- Target: imported `stm32f412`; system: `configs/systems/onboarding/stm32f412.yaml`.
- Base commit: `fea3d01875289febdcc5df59fc658cb6d32312cf`; pre-fix working diff SHA-256: `ffdfc9e2c7d4311a5953cccef6948c9d979d9fe2a8e0dc8eb0de6e402cec6088`.
- Firmware: `firmware-stm32f429-smoke`, SHA-256 `b72971c405de1f2393dea621ed654b68fe8ec06ecf5f16692a14a7c403f9f0c2`.
- Command: `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 PATH=/usr/bin:/home/andrii/.cargo/bin:/usr/local/bin:/bin /usr/bin/python3 scripts/run_renode_onboarding_inventory.py --only stm32f412`.
- Outcome: **FAIL** at `run_smoke`, after a real 800,000-instruction boot to `max_steps`.
- Original assertion: UART contains `adc1 in1=000003FF gated=1 clock=1 eoc=1 ok`.
- Observed UART: `adc1 in1=00000000 gated=0 clock=0 eoc=0 FAIL`.
- Raw logs, JUnit, result, snapshot, UART, and metrics are alongside this record.

## Candidate causes

1. **RCC profile omitted.** The F412 RCC was declared as `stm32f4_rcc` without `config.profile: stm32f4`; APB2ENR bit 8 did not read back as enabled. This matches `clock=0`. A previous F429 smoke exposed the same omission and passed once its RCC profile was explicit. Weak link: this smoke did not separately log the RCC read.
2. **ADC clock binding/profile or fixture route is wrong.** F412 declares APB2ENR bit 8, the STM32F4 ADC profile, PA1/IN1, and a 25% potentiometer fixture, but these paths were not independently observed because RCC enable failed first.

Cheapest discriminating test: set only the F412 RCC profile to `stm32f4` and rerun the original target-only smoke; ADC pass supports candidate 1, while unchanged `clock=0` kills it and leaves candidate 2.

## Discriminating rerun and closure

- Changed only the F412 RCC configuration after the frozen failure: added `config.profile: stm32f4`.
- Re-ran the same command and the same smoke assertions at 2026-11-17 05:15–05:17 UTC. It booted 800,000 instructions to `max_steps` and passed. UART: `adc1 in1=000003FF gated=1 clock=1 eoc=1 ok`.
- This confirms candidate 1 and rejects candidate 2 for this measured fixture. The original failure remains preserved above as the negative control; the original acceptance assertion was not weakened.
- After-fix chip SHA-256: `8a1c1e38c2692bb702fd2aa97f4ae00c0108c4d1f107131564b009b702964d6d`. Firmware SHA-256 stayed `b72971c405de1f2393dea621ed654b68fe8ec06ecf5f16692a14a7c403f9f0c2`.
- Remaining gaps: this is repository CLI smoke evidence for this ADC path, not a hosted `labwired_verify` result or hardware confirmation; other F412 peripheral behavior is outside this smoke.

## Hashes before the discriminating rerun

- chip: `0540c215d58d727d2ecd170c6373da6b0ebb5fe00f73b9c4f71ce5b73ccbb412`
- system: `478b33a7e92cb5a67609c30c8f6e15608184941cb3b0f6d5073cee824a234113`
- smoke: `b45e6f57ef45ce0189e4b042613056cd310e63e584d8df6aeb3d95be63b54ae8`
- runner: `df8ffe07bf4e345f1e7d864c470f1117b1da0c8d9969b23ea3ebeb6089f5d1ef`
- firmware manifest: `364895ed8e773680db833881beba0a4382b5697d01a3165322639033b1e2635f`
- firmware source: `517d51c69479b7fef306615e226ab97920b479fd3c88688258b89a003fb7a510`
