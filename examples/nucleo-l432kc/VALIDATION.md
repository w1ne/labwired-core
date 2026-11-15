# NUCLEO-L432KC validation receipt

Recorded 2026-11-15 on branch `onboarding/nucleo-l432kc-20261115`.

## PR-gated raw-register smoke

```sh
./examples/nucleo-l432kc/firmware/build.sh
labwired test --script examples/nucleo-l432kc/io-smoke.yaml \
  --output-dir out/nucleo-l432kc-smoke --no-uart-stdout
```

- Result: `PASS 3/3 checks · io-smoke · 150000 steps`.
- `uart_contains: STM32L432KC_OK` (USART2 TDR via PA2 AF7).
- `gpio_edges: gpiob:3 ≥ 4` (LD3).
- `expected_stop_reason: max_steps` (no simulation error).
- Firmware: `tests/fixtures/board-smoke/stm32l432kc.elf`, built by
  `firmware/build.sh` (arm-none-eabi-gcc, `-Os`, no libc). sha256
  `d39d9b3d37435cac4925a6b5a949b347d8357aacb567dddcf11e0fdaa51fbc8a`.
- Negative control: the same ELF on a chip copy whose `uart2` clock gate is
  APB1ENR1 bit 18 instead of 17 → `FAIL 1/3`, zero UART bytes captured. The
  USART2 enable gate is enforced, so the banner is not free.
- CI: `configs/ci/boards.yml` entry `nucleo-l432kc` (`sim-validate`,
  `gate: true`).

## Stock STM32duino Arduino matrix row

9 pass / 0 skip / 0 fail. Detail, step counts, the L6 waveform and the L5
controls: [`evidence/arduino-matrix/README.md`](evidence/arduino-matrix/README.md).
CI lane: `core-arduino-matrix-smoke` (nightly and on push to `main`, not on
PRs).

## Not proven

- No NUCLEO-L432KC silicon run, register diff or reset-value capture.
  DBGMCU REV_ID is a placeholder 0.
- USB FS device, CRS, OPAMP, SWPMI and FIREWALL are not modelled and not
  declared. SYSCFG is a read-zero stub (no EXTICR routing).
- The ADC model has no settling time, noise or sampling-time effect; the
  1650 mV level is a static board input, not a circuit node.
