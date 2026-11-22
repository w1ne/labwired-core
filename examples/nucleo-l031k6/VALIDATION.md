# NUCLEO-L031K6 validation receipt

Recorded 2026-11-21 10:05 CET (Europe/Berlin) on branch
`feat/stm32l031k6-nucleo-l031k6`.

## PR-gated raw-register smoke

```sh
./examples/nucleo-l031k6/firmware/build.sh
labwired test --script examples/nucleo-l031k6/io-smoke.yaml \
  --output-dir out/nucleo-l031k6-smoke --no-uart-stdout
```

- Result: `PASS  3/3 checks · io-smoke · 150000 steps`.
- `uart_contains: STM32L031K6_OK` (USART2 TDR via PA2 AF4). Captured
  sink is in `evidence/io-smoke/uart.log`.
- `gpio_edges: gpiob:3 ≥ 4` (LD3).
- `expected_stop_reason: max_steps` (no simulation error).
- Firmware: `tests/fixtures/board-smoke/stm32l031k6.elf`, built by
  `firmware/build.sh` (arm-none-eabi-gcc, Cortex-M0+, `-Os`, no libc). sha256
  `beb7dc7702c5f55af4faf431a111da846ca4805c5a975968d8d39be414afb608`.
- Negative control (not committed): the same ELF on a chip copy whose
  `uart2` clock gate is APB1ENR bit 18 instead of 17 captured zero UART
  bytes and failed `uart_contains`. The USART2 enable gate is enforced, so
  the banner is not free. In that budget the firmware was still waiting on
  TXE, so LD3 edges also did not appear; that does not weaken the UART
  result.
- CI: `configs/ci/boards.yml` entry `nucleo-l031k6` (`sim-validate`,
  `gate: true`).

## Not proven

- No NUCLEO-L031K6 silicon run, register diff or reset-value capture.
- No Arduino matrix row has been run for this chip, so SPI1 and I2C1 are
  not bus-proof proven.
- USB, LCD, DAC and LPTIM are not on this die and are not declared.
- SYSCFG uses the real STM32L0 model (`Stm32L0Syscfg`); this smoke does not
  exercise it. PWR, FLASH, ADC, DMA, SPI and I2C are modelled and are also
  not driven by this firmware.
