# NUCLEO-L452RE-P onboarding smoke

The smoke boots an STM32L452RE profile and exercises two paths wired on the
board: LPUART1 output through the default ST-LINK Virtual COM Port, and the
active-high LD4 user LED on PB13. ST's UM2206 documents the VCP on PA2/PA3 and
the LED on PB13.

The narrow descriptor exposes RCC, GPIOA, GPIOB, SysTick and LPUART1. It does
not claim USB, ST-LINK protocol behavior, the user button, or other MCU
peripherals. The VCP is represented by the LPUART byte sink; the USB bridge is
not simulated.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-l452re-p \
  --crate firmware-nucleo-l452re-p-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l452re-p/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l452re-p-smoke.yaml \
  --out-dir examples/nucleo-l452re-p/evidence
```

The committed build and simulation evidence is in `evidence/`. The live ST
status check is recorded in `evidence/vendor-status.md`.

## Sources

- ST [UM2206](https://www.st.com/resource/en/user_manual/dm00387966.pdf), §§6.9-6.10: default LPUART1 VCP on PA2/PA3 and LD4 on PB13.
- ST [STM32L452xx datasheet DS11912](https://www.st.com/resource/en/datasheet/stm32l452re.pdf): memory, Cortex-M4, and LPUART1 alternate function 8 on PA2/PA3.
- ST [RM0394](https://www.st.com/resource/en/reference_manual/rm0394-stm32l41xxx42xxx43xxx44xxx45xxx46xxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): RCC APB1ENR2, GPIO and LPUART register maps.
- ST product status and store availability: [`evidence/vendor-status.md`](evidence/vendor-status.md).
