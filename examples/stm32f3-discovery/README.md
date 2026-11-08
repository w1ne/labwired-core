# STM32F3DISCOVERY LD3 smoke

This system maps the STM32F303VCT6 and only its onboard user LED LD3 on PE9.
The firmware boots on the Cortex-M4, enables GPIOE, configures PE9 as an
output, and toggles it. The CLI smoke requires at least eight recorded GPIO
edges.

## Run

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f3-discovery \
  --crate firmware-stm32f3-discovery-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f3-discovery/io-smoke.yaml \
  --system configs/systems/stm32f3-discovery.yaml \
  --out-dir examples/stm32f3-discovery/evidence
```

The committed `evidence/logs/run_smoke.log` and `evidence/simulation-output/`
record the local LabWired CLI result. Current ST product status is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The descriptor includes the reset-clock gate for GPIOE and the GPIOE model
needed for PE9. It omits the other LEDs, user button, USB, ST-LINK/VCP,
L3GD20/I3G4250D gyroscope, and LSM303DLHC/LSM303AGR e-compass. The separate
8 KiB CCM RAM is not mapped. This smoke does not claim those features or a
physical ST-LINK connection.
