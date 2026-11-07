# NUCLEO-G431RB onboarding smoke

This profile boots bare-metal firmware on the STM32G431RBT6 and exercises two
board-connected paths: LPUART1 output through the default STLINK-V3E virtual COM
port, and LD2 on PA5. ST's UM2505 documents PA2/PA3 as the default LPUART1 VCP
route and PA5 as the active-high user LED.

The smoke descriptor exposes RCC, GPIOA and LPUART1 only. It does not claim the
board button, the remaining MCU peripherals, USB behavior, or other board
features. Each exposed peripheral is modeled; none is a zero-answering stub.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g431rb \
  --crate firmware-nucleo-g431rb-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g431rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-g431rb.yaml \
  --out-dir examples/nucleo-g431rb/evidence
```

The committed build and simulation logs are in `evidence/`. The live vendor
status receipt is `evidence/vendor-status.md`.

## Sources and model boundary

- ST [UM2505](https://www.st.com/resource/en/user_manual/dm00556337-stm32g4-nucleo64-boards-mb1367-stmicroelectronics.pdf), §§6.6.1, 6.6.4 and 8: LD2 PA5, default LPUART1 VCP on PA2/PA3, and board pin assignments.
- ST [STM32G431 datasheet DS12589](https://www.st.com/resource/en/datasheet/stm32g431rb.pdf): PA2/PA3 LPUART1 alternate function 12.
- ST [RM0440](https://www.st.com/resource/en/reference_manual/dm00355726-stm32g4-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): G4 RCC, GPIO and LPUART register maps.
- ST's live product status and authorized distributor stock: [`evidence/vendor-status.md`](evidence/vendor-status.md).

This is simulator smoke evidence for the declared LPUART1 and LD2 paths. It is
not a silicon capture or a claim about omitted board features.
