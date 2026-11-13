# NUCLEO-F446RE validation — 2026-11-13

## Sources

- ST [STM32F446RE product/DS10693](https://www.st.com/en/microcontrollers-microprocessors/stm32f446re.html):
  512 KiB flash, 128 KiB SRAM, max 180 MHz Cortex-M4F.
- ST [RM0390](https://www.st.com/resource/en/reference_manual/dm00135183.pdf):
  F446 peripheral memory map, RCC and GPIO.
- ST [CMSIS device header](https://github.com/STMicroelectronics/cmsis_device_f4/blob/master/Include/stm32f446xx.h):
  `TIM2_BASE=0x40000000`, `USART2_BASE=0x40004400`, `GPIOA_BASE=0x40020000`,
  `GPIOC_BASE=0x40020800`, `RCC_BASE=0x40023800`,
  `TIM2_IRQn=28`, `USART2_IRQn=38`.
- ST UM1724 Nucleo-64 board manual: LD2 PA5, B1 PC13, USART2 VCP PA2/PA3.

## Local evidence

Command from repository root:

```sh
./examples/nucleo-f446re/firmware/build.sh
labwired test --script examples/nucleo-f446re/io-smoke.yaml \
  --output-dir out/nucleo-f446re-smoke --no-uart-stdout
labwired run --chip configs/chips/stm32f446re.yaml \
  --firmware tests/fixtures/board-smoke/stm32f446re.elf --max-steps 10000
```

- Compiler: `arm-none-eabi-gcc`, Cortex-M4 Thumb, soft-float ABI; no LabWired dependencies.
- ELF SHA-256: `cd3f0a78d94eb8d935caabf2b7e9623ed98feb9d92097e9b6759eb434e671293`.
- CLI: `labwired-cli 0.25.0` (2026-11-13, local installed binary).
- Result: `PASS 3/3`, 150000 steps, 0.88 s as reported by the CLI.
- UART: `STM32F446RE_OK`; GPIO PA5 edge count passes, stop reason `max_steps`.

This receipt covers the minimal executable subset. Earlier source-build evidence
for stock STM32duino is recorded separately in `evidence/arduino-matrix/`; it
does not establish stock STM32Cube HAL/Zephyr operation, whole-chip parity,
physical silicon reset parity or analog/camera/USB function. Hardware matching
and stronger conformance evidence remain required before an L2/L3 claim.
