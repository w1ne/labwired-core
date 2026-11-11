# Validation record

On 2026-11-11, `scripts/onboarding_smoke.sh` built the CLI and firmware, then
ran `io-smoke.yaml` against `configs/systems/nucleo-u3c5zi-q.yaml` in the local
repository simulator. The structured result passed all three assertions:

- USART1 output contains `U3C5_USART1_OK`.
- GPIOA pin 5 produces at least eight edges.
- The bounded run stops at 200,000 steps.

The command logs and structured result are in [evidence](evidence/), including
the committed [run log](evidence/logs/run_smoke.log). This is local simulator
evidence. The hosted `labwired_verify` board catalog currently has no
NUCLEO-U3C5ZI-Q entry, so no hosted `model_verified` verdict is claimed.

The smoke uses ST's documented board mapping from [UM3599](https://www.st.com/resource/en/user_manual/um3599-stm32u3-nucleo144-board-mb2222-stmicroelectronics.pdf):
USART1 virtual COM on PA9/PA10 and green LD1 on PA5. The physical ST-LINK
bridge and pad-level serial waveform are not simulated.
