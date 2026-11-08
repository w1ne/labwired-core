# STM32H735G-DK

This focused board profile boots firmware on the fitted STM32H735IGK6 and
exercises USART3 output through the onboard ST-LINK VCP plus both active-low
user LEDs (LD1 on PC3 and LD2 on PC2). The board profile only declares those
tested paths.

## Run

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id stm32h735g-dk \
  --crate firmware-stm32h735g-dk-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32h735g-dk/onboarding-smoke.yaml \
  --system configs/systems/stm32h735g-dk.yaml \
  --out-dir examples/stm32h735g-dk/evidence

python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/stm32h735g-dk/evidence \
  --markdown-out examples/stm32h735g-dk/evidence/onboarding-scoreboard.md \
  --json-out examples/stm32h735g-dk/evidence/onboarding-scoreboard.json
```

The committed local simulator logs and result are under [`evidence/`](evidence/).
This local smoke does not represent a physical-board capture. The separate
hosted verifier currently returns `unsupported` for this H735 run; its fault
record is in [`evidence/hosted-verification.md`](evidence/hosted-verification.md).

## Pin map and model scope

| Board path | MCU pins | Model path |
|---|---|---|
| USART3 to ST-LINK VCP | PD8 TX, PD9 RX (AF7) | H7 RCC gate, GPIOD, USART3 |
| Green user LED LD1 | PC3, active-low | H7 RCC gate, GPIOC |
| Red user LED LD2 | PC2, active-low | H7 RCC gate, GPIOC |

The exact STM32H735IGK6 profile maps 1 MiB Flash and the 128 KiB DTCM used by
this image. The fitted MCU has additional SRAM, which is not mapped by this
smoke profile. LCD/touch, Ethernet, USB, audio, SD, external Flash/HyperRAM,
and other MCU peripherals are outside the declared scope. No zero-answering
stubs are used by this profile.

## Sources

- ST [STM32H735G-DK product page](https://www.st.com/en/evaluation-tools/stm32h735g-dk.html)
  and [live status receipt](evidence/vendor-status.md).
- ST [UM2679 Rev 5](https://www.st.com/resource/en/user_manual/dm00682073-discovery-kit-with-stm32h735ig-mcu-stmicroelectronics.pdf): fitted MCU,
  memory, LED polarity and pins, VCP wiring, and board peripherals.
- ST [STM32H7 reference manual RM0468](https://www.st.com/resource/en/reference_manual/rm0468-stm32h745755-and-stm32h747757-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): RCC/GPIO/USART registers.
- ST [STM32H735G-DK board support package](https://github.com/STMicroelectronics/stm32h735g-dk-bsp): board LED active polarity and COM1/USART3 path.
