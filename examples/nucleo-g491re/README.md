# NUCLEO-G491RE onboarding smoke

Demand context: NUCLEO/Discovery kits rank 5 in the target list, at 23% of
surveyed main-project kit use. This change extends the existing NUCLEO-G491RE
profile with FDCAN1; it does not add a new board or MCU variant.

This focused board profile boots bare-metal firmware on the STM32G491RET6 and
exercises three paths: LPUART1 output through the default ST-LINK virtual COM
port, LD2 on PA5, and an FDCAN1 frame through internal loopback. The CAN check
verifies frame ID `0x321`, DLC 8, and all eight payload bytes in RX FIFO0. ST
UM2505 documents the default PA2/PA3 VCP route and the active-high PA5 LED.

The chip descriptor exposes modeled RCC, GPIOA, LPUART1, and FDCAN1. PA11/PA12
are mapped to FDCAN1 RX/TX, but the board has no fitted CAN transceiver. The
smoke uses internal loopback and does not claim pin-level CAN output, bus
arbitration, bit timing, or external-node behavior. Other MCU peripherals, the
user button, USB, and the remainder of the board are omitted. None of the
declared peripherals is a zero-answering stub.

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
- ST [STM32G491RE datasheet DS13122](https://www.st.com/resource/en/datasheet/stm32g491re.pdf), Table 13: PA11/PA12 FDCAN1 alternate functions (AF9); [RM0440](https://www.st.com/resource/en/reference_manual/dm00355726-stm32g4-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): FDCAN1 register and message-RAM map.
- ST [NUCLEO-G491RE product page](https://www.st.com/en/evaluation-tools/nucleo-g491re.html) and live eStore listing: current status and stock evidence are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
- ST [STM32G491RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32-32-bit-arm-cortex-mcus/stm32-mainstream-mcus/stm32g4-series/stm32g4x1/stm32g491re.html) and [RM0440](https://www.st.com/resource/en/reference_manual/dm00355726-stm32g4-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): active part details and G4 register map.
- ST's live part status and stock evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).

This is simulator smoke evidence for the declared LPUART1 and LD2 paths, not a
silicon capture or a claim about omitted board features.
