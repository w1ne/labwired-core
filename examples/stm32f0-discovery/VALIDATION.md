# STM32F0DISCOVERY validation

The onboarding smoke builds bare-metal Cortex-M0 firmware and runs it against
the narrow `stm32f0-discovery` system profile.

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f0-discovery \
  --crate firmware-stm32f0-discovery-smoke \
  --target thumbv6m-none-eabi \
  --script examples/stm32f0-discovery/onboarding-smoke.yaml \
  --system configs/systems/stm32f0-discovery.yaml \
  --out-dir examples/stm32f0-discovery/evidence
```

Acceptance requires at least eight recorded output edges on GPIOC pin 9 and a
normal stop at the instruction limit. The firmware enables RCC AHBENR.IOPCEN,
sets PC9 to output mode, and toggles LD3 through GPIOC BSRR.

ST's [UM1525](https://www.st.com/resource/en/user_manual/um1525-stm32f0discovery-discovery-kit-for-stm32-f0-microcontrollers-stmicroelectronics.pdf)
identifies the STM32F051R8T6 and maps green user LED LD3 to PC9. ST's
[RM0091](https://www.st.com/resource/en/reference_manual/rm0091-stm32f0x1stm32f0x2stm32f0x8-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
defines the RCC and GPIO register map used by this minimal profile.
