# NUCLEO-L432KC onboarding smoke

This focused profile boots bare-metal firmware on the STM32L432KC and exercises
two board-connected paths: USART2 output through the default ST-LINK virtual
COM port, and the active-high LD3 user LED on PB3. ST's UM1956 documents the
VCP route on PA2/PA15 and LD3 on PB3.

The profile exposes only modeled RCC, GPIOA, GPIOB, and USART2. The reset button,
USB/ST-LINK internals, and the remaining MCU peripherals are omitted. The
declared peripheral types use the engine's real models; none is a zero-answering
stub.

Run the local simulator smoke and refresh the committed evidence with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-l432kc \
  --crate firmware-nucleo-l432kc-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l432kc/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l432kc.yaml \
  --out-dir examples/nucleo-l432kc/evidence
```

The live vendor-status receipt is `evidence/vendor-status.md`; build and local
simulator logs are under `evidence/logs/` and `evidence/simulation-output/`.

## Hardware sources and scope

- ST [UM1956 Rev 6](https://www.st.com/resource/en/user_manual/um1956-stm32-nucleo32-boards-mb1180-stmicroelectronics.pdf), §§7.8–7.9: LD3 is on PB3 and the default VCP path uses PA2 TX / PA15 RX.
- ST [STM32L432KC datasheet DS11451](https://www.st.com/resource/en/datasheet/stm32l432kc.pdf): 256 KiB Flash, 64 KiB SRAM, and the PA2/PA15 alternate functions.
- ST [RM0394](https://www.st.com/resource/en/reference_manual/rm0394-stm32l41xxx42xxx43xxx44xxx45xxx46xxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): STM32L4 register map and RCC/GPIO/USART2 register layout.
- ST's live product and stock check: [`evidence/vendor-status.md`](evidence/vendor-status.md).

This is local simulator smoke evidence for the declared USART2 and LD3 paths.
It does not claim hosted `labwired_verify` or physical hardware capture, and it
does not cover the omitted board functions.
