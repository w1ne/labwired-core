# Nordic nRF54LM20 DK — LED0 and VCOM smoke

This profile runs firmware for the **nRF54LM20A emulation target** supported by
the nRF54LM20 DK. The physical DK carries the nRF54LM20B; the board uses the
same Nordic-supported target selection documented by the nRF Connect SDK.

The smoke boots the Cortex-M33 application core, transmits a marker through
UARTE20, and toggles the board's active-high LED0 on P1.22. The scoped profile
maps only application RRAM/SRAM, UARTE20, and GPIO P1. Radio, nPM1300 PMIC,
J-Link interface MCU, external 8 MB flash, RF/NFC hardware, and other MCU
peripherals are not modeled by this smoke. The PMIC's default LED supply is
assumed; PMIC configuration and LED power switching are not simulated.

Run the full local smoke and generate its evidence with:

```sh
scripts/onboarding_smoke.sh \
  --target-id nrf54lm20dk-led-smoke \
  --crate firmware-nrf54lm20dk-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nrf54lm20dk/onboarding-smoke.yaml \
  --system configs/systems/nrf54lm20dk-led-smoke.yaml \
  --out-dir examples/nrf54lm20dk/evidence
```

See [live vendor availability](evidence/vendor-status.md) and
[verification scope](VALIDATION.md).
