# STM32F3DISCOVERY validation

The onboarding smoke builds bare-metal Cortex-M4 firmware and runs it against
the narrow `stm32f3-discovery` system profile.

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f3-discovery \
  --crate firmware-stm32f3-discovery-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f3-discovery/io-smoke.yaml \
  --system configs/systems/stm32f3-discovery.yaml \
  --out-dir examples/stm32f3-discovery/evidence
```

Acceptance requires at least eight output edges on GPIOE pin 9 and a normal
stop at the configured instruction limit. The firmware accesses the RCC
AHBENR GPIOE clock gate and GPIOE MODER/BSRR only.

Source grounding: [ST UM1570](https://www.st.com/resource/en/user_manual/um1570-discovery-kit-with-stm32f303vc-mcu-stmicroelectronics.pdf)
maps user LD3 to PE9; the [ST product page](https://www.st.com/en/evaluation-tools/stm32f3discovery.html)
identifies the board and STM32F303VCT6.
