# STM32F429I-DISC1 GPIO smoke

The smoke boots a Cortex-M4 image on the STM32F429ZIT6 and toggles the board's
active-high green user LED LD3 on PG13. The local simulator check requires at
least 16 GPIO transitions before its instruction limit.

ST's live product page lists the STM32F429I-DISC1 as Active and in volume
production. ST's eStore listed it In Stock with Add to Cart enabled during the
2026-11-04 check. The board, MCU and vendor evidence are in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f429i-disc1 \
  --crate firmware-stm32f429i-disc1-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f429i-disc1/onboarding-smoke.yaml \
  --system configs/systems/stm32f429i-disc1.yaml \
  --out-dir examples/stm32f429i-disc1/evidence
```

The committed evidence contains the build logs, asserted smoke result, simulator
artifacts and onboarding metrics.

## Model boundary

The chip profile declares only RCC, SysTick and GPIOG for this LED path. It
does not declare the board's 2.4-inch display, external SDRAM, I3G4250D
gyroscope, USB OTG, buttons or ST-LINK/VCP. No zero-answering stubs are used.
This is local simulator evidence; the hosted `labwired_verify` target list does
not currently include STM32F429, and this is not physical-hardware evidence.
