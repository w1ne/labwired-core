# NUCLEO-G431KB onboarding smoke

The narrow profile boots bare-metal firmware and exercises the board's default
USART2 path to the ST-LINK Virtual COM Port. ST UM2397 identifies the board's
target and routes USART2 PA2/PA3 to the VCP with the default solder bridges.
The firmware enables GPIOA and USART2, configures PA2/PA3 for USART2, and
prints `NUCLEO_G431KB_OK`.

The descriptor exposes RCC, GPIOA and USART2 only. It does not claim the user
LED, board button, other MCU peripherals, or USB behavior. None of the
peripherals in this smoke descriptor are zero-answering stubs.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g431kb \
  --crate firmware-nucleo-g431kb-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g431kb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-g431kb-smoke.yaml \
  --out-dir examples/nucleo-g431kb/evidence
```

The committed run log and full simulation output are in `evidence/`. The live
vendor-status receipt is `evidence/vendor-status.md`.

## Source and model boundary

- ST [UM2397](https://www.st.com/resource/en/user_manual/um2397-stm32g4-nucleo32-board-mb1430-stmicroelectronics.pdf), §§1 and 6.6.4: board target and VCP wiring.
- ST [RM0440](https://www.st.com/resource/en/reference_manual/dm00355726-stm32g4-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): G4 register layout used by the narrow modeled path.
- ST's live product and stock state: [`evidence/vendor-status.md`](evidence/vendor-status.md).

This is simulator smoke evidence for the declared USART2 path. It is not a
silicon capture or a claim about the omitted board functions.
