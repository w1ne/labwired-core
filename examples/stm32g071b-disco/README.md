# STM32G071B-DISCO onboarding smoke

This board profile boots bare-metal firmware and exercises USART3 output to
the ST-LINK/V2-1 virtual COM port. ST's UM2401 identifies USART3 (or USART4)
as the VCP and maps USART3 TX/RX to PB8/PB9 on the default CN1 assignment.
The smoke enables the G0 GPIOB and USART3 clocks, configures PB8/PB9 for AF0,
and prints `STM32G071B_DISCO_OK`.

The dedicated profile exposes only the MCU core, RCC, GPIOB, and USART3. The
OLED, USB-C/Power Delivery path, current monitors, joystick, reed sensor, and
ST-LINK implementation are outside the model. No peripheral in this profile
uses a zero-answering stub.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32g071b-disco \
  --crate firmware-stm32g071b-disco-smoke \
  --target thumbv6m-none-eabi \
  --script examples/stm32g071b-disco/onboarding-smoke.yaml \
  --system configs/systems/stm32g071b-disco.yaml \
  --out-dir examples/stm32g071b-disco/evidence
```

The committed local smoke log and simulator outputs are in `evidence/`. The
live purchasability receipt is `evidence/vendor-status.md`.

## Sources and model boundary

- ST [UM2401](https://www.st.com/resource/en/user_manual/dm00496511-stm32g0-discovery-kit-for-usb-type-c-and-power-delivery-stmicroelectronics.pdf), §§9.14 and 10.1: VCP and PB8/PB9 assignment.
- ST [RM0444](https://www.st.com/resource/en/reference_manual/dm00371828-stm32g0x1-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): STM32G0 RCC, GPIO, and USART register maps.
- ST's live board status and stock: [`evidence/vendor-status.md`](evidence/vendor-status.md).

This is simulator smoke evidence for the declared USART3 path. It is not a
silicon capture or a claim about the omitted board features.
