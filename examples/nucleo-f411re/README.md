# NUCLEO-F411RE

This Nucleo-64 board uses ST's STM32F411RET6 (Cortex-M4F, 512 KiB flash,
128 KiB SRAM). The board profile maps LD2 to PA5 and the on-board ST-LINK
Virtual COM Port to USART2 on PA2/PA3. The smoke prints
`NUCLEO_F411RE_OK` over USART2 and records at least eight LD2 transitions.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f411re \
  --crate firmware-nucleo-f411re-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f411re/io-smoke.yaml \
  --system configs/systems/nucleo-f411re.yaml \
  --out-dir examples/nucleo-f411re/evidence
```

Committed build and simulator logs are under [`evidence/`](evidence/).

## Sources and model boundary

The board mapping follows ST's [NUCLEO-F411RE product page](https://www.st.com/en/evaluation-tools/nucleo-f411re.html)
and [UM1724 Nucleo-64 board manual](https://www.st.com/resource/en/user_manual/um1724-stlinkv21-in-circuit-debuggerprogrammer-for-stm8-and-stm32-stmicroelectronics.pdf).
The memory and MCU limits follow the [STM32F411RE datasheet](https://www.st.com/resource/en/datasheet/stm32f411re.pdf).

The chip profile exposes only RCC, GPIOA, and USART2, with PA2, PA3, and PA5
named. It has no zero-answering stub peripherals. The smoke exercises the RCC
clock gates, USART2 output, and GPIOA/PA5. The user button,
USB, DMA, and all other peripheral paths are omitted and unsupported. USART2
bytes are captured at the simulator peripheral; ST-LINK firmware, USB, and a
physical VCP link are not modeled. No hardware capture is claimed.

ST's live status and availability record is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
