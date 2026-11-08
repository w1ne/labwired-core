# NUCLEO-F303K8 validation

The committed run under `evidence/` was produced by `scripts/onboarding_smoke.sh`
on 2026-11-04. The local engine smoke passed all three assertions at 2,200
steps:

- USART2 output contains `NUCLEO_F303K8_OK`.
- GPIOB/PB3 has at least eight transitions; the run recorded 11.
- The run stops at the configured `max_steps` limit.

The firmware enables the reset-clock GPIOA/GPIOB and USART2 gates, configures
PA2 for USART2 transmit, and toggles PB3 for LD3. The smoke uses the modeled
RCC gate registers, SysTick, GPIOA/GPIOB and USART2.

## Model boundary

No explicit zero-answering peripheral stub is used in this profile. The other
STM32F303K8 blocks are omitted from the descriptor and remain unmodeled. The
local engine observes USART2 bytes at the MCU peripheral; it does not simulate
the ST-LINK/V2-1 USB bridge or prove electrical routing through the board's
solder bridges. The smoke also does not claim button, ADC, timer, DMA, USB,
CCM SRAM, or broader RCC behavior.

ST's active/buyable status and the board manual sources used for the narrow
pin map are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
