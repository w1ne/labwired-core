# STM32F3348-DISCO GPIO smoke

The model boots a bare Cortex-M4 image on the STM32F334C8T6 and toggles the
board's active-high red user LED LD3 on PB6. The acceptance check requires at least 16
GPIO transitions before the simulator step limit.

ST lists the STM32F3348-DISCO as Active and In Stock and the STM32F334C8T6 as
Active/in volume production. The live check and board references are recorded
in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f3348-disco \
  --crate firmware-stm32f3348-disco-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f3348-disco/onboarding-smoke.yaml \
  --system configs/systems/stm32f3348-disco.yaml \
  --out-dir examples/stm32f3348-disco/evidence
```

The committed evidence includes build logs, the smoke result, simulator
artifacts, and onboarding metrics.

## Model boundary

This profile declares only the reset-clock RCC gate and the real GPIOB model
needed for PB6/LD3. It omits LD4/LD5/LD6, the user button, the high-brightness
LED buck converter, ST-LINK/VCP, and all other MCU peripherals. No zero-answering
stub windows are declared. This is local simulator evidence; no physical
STM32F3348-DISCO capture is included.
