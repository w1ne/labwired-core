# NUCLEO-G491RE onboarding smoke

This focused board profile boots bare-metal firmware on the STM32G491RET6 and
exercises two board-connected paths: LPUART1 output through the default ST-LINK
virtual COM port, and LD2 on PA5. ST UM2505 documents the default PA2/PA3 VCP
route and the active-high PA5 user LED.

The chip descriptor exposes only modeled RCC, GPIOA, and LPUART1. Other MCU
peripherals, the user button, USB, and the remainder of the board are omitted
from this profile. None of the declared peripherals is a zero-answering stub.

Reproduce the smoke and refresh its evidence with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g491re \
  --crate firmware-nucleo-g491re-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g491re/onboarding-smoke.yaml \
  --system configs/systems/nucleo-g491re.yaml \
  --out-dir examples/nucleo-g491re/evidence
```

The committed build and simulation logs are under `evidence/logs/` and
`evidence/simulation-output/`. The live vendor status receipt is
`evidence/vendor-status.md`.

## Sources and model boundary

- ST [UM2505](https://www.st.com/resource/en/user_manual/dm00556337-stm32g4-nucleo64-boards-mb1367-stmicroelectronics.pdf), §§6.6.1 and 6.6.5: LD2 on PA5 and the default LPUART1 VCP route on PA2/PA3.
- ST [STM32G491RE datasheet DS13122](https://www.st.com/resource/en/datasheet/stm32g491re.pdf), Table 13: PA2/PA3 LPUART1 alternate functions (AF12), and the MCU memory sizes.
- ST [STM32G491RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32-32-bit-arm-cortex-mcus/stm32-mainstream-mcus/stm32g4-series/stm32g4x1/stm32g491re.html) and [RM0440](https://www.st.com/resource/en/reference_manual/dm00355726-stm32g4-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): active part details and G4 register map.
- ST's live part status and stock evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).

This is simulator smoke evidence for the declared LPUART1 and LD2 paths, not a
silicon capture or a claim about omitted board features.
