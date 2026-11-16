# NUCLEO-L432KC — STM32L432KC on the STM32L4 models

The STM32L432KC (Arm Cortex-M4F, 80 MHz, 256 KiB flash, 64 KiB SRAM,
UFQFPN32) is modelled with its own instance set and memory map from ST's
`stm32l432xx.h` (RM0394, L43x), and reuses the STM32L4 family register models.
The Nucleo-32 board has LD3 on PB3 (Arduino D13) and USART2 on PA2/PA15 to the
ST-LINK virtual COM port.

**Scope of the PR-gated smoke:** ARM reset, valid SRAM stack, RCC AHB2/APB1
enables, PA2 AF7, USART2 TX, and LED edges on PB3. The stock STM32duino
Arduino matrix row (L0–L8) is in `evidence/arduino-matrix/`. USB FS device,
CRS, OPAMP, SWPMI and FIREWALL are not modelled and not declared. No physical
hardware comparison has been made.

## Reproduce

From the labwired-core repository root:

```sh
./examples/nucleo-l432kc/firmware/build.sh
labwired test --script examples/nucleo-l432kc/io-smoke.yaml \
  --output-dir out/nucleo-l432kc-smoke --no-uart-stdout
```

Expected: `3/3` passing (UART contains `STM32L432KC_OK`; PB3 at least four
edges; reaches the step budget without a simulation error).

The stock Arduino row:

```sh
python3 validation/arduino-matrix/run_matrix.py --boards stm32l432kc
```

The ELF in `tests/fixtures/board-smoke/stm32l432kc.elf` is committed so users
with only an installed CLI can run it without an ARM compiler. Its source is
`firmware/main.c` and `firmware/linker.ld`.
See `VALIDATION.md` for the recorded evidence.
