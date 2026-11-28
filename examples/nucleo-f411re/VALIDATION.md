# NUCLEO-F411RE validation receipt

Recorded 2026-11-15 on branch `onboarding/nucleo-f411re-20261115`.

## PR-gated raw-register smoke

```sh
./examples/nucleo-f411re/firmware/build.sh
labwired test --script examples/nucleo-f411re/io-smoke.yaml \
  --output-dir out/nucleo-f411re-smoke --no-uart-stdout
```

- Result: `PASS 3/3 checks · io-smoke · 150000 steps`.
- `uart_contains: STM32F411RE_OK` (USART2 DR via PA2 AF7, HSI 16 MHz, BRR 139).
- `gpio_edges: gpioa:5 ≥ 4` (LD2).
- `expected_stop_reason: max_steps` (no simulation error).
- Firmware: `tests/fixtures/board-smoke/stm32f411re.elf`, built by
  `firmware/build.sh` (arm-none-eabi-gcc, `-Os`, no libc). sha256
  `36d23ea88caf792d17585bae184ed809e0588193938a53da7beb54817a05804a`.
- Negative control: the same ELF on a chip copy with the USART2 window moved
  from 0x40004400 to 0x40004800 → `FAIL 0/3`, zero UART bytes captured.
- CI: `configs/ci/boards.yml` entry `nucleo-f411re` (`sim-validate`,
  `gate: true`).

## Stock STM32duino Arduino matrix row

8 pass / 1 skip (L8, no CAN on F411) / 0 fail. Detail, step counts, the L6
waveform and the L5 controls:
[`evidence/arduino-matrix/README.md`](evidence/arduino-matrix/README.md).
CI lane: `core-arduino-matrix-smoke` (nightly and on push to `main`, not on
PRs).

## Not proven

- No NUCLEO-F411RE silicon run, register diff or reset-value capture.
  DBGMCU is a stub with no IDCODE.
- DMA1/DMA2, USB OTG FS, SDIO, CRC, WWDG, I2S2EXT/I2S3EXT are zero-answering
  stubs. SYSCFG has no EXTICR routing.
- Clock gating is enforced only for TIM2, ADC1, SPI1 and SPI5 (same as the
  Black Pill descriptor).
- The ADC model has no settling time, noise or sampling-time effect; the
  1650 mV level is a static board input, not a circuit node.
