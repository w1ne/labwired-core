# NUCLEO-F411RE — STM32F411RE on the STM32F4 models

The STM32F411RET6 (Arm Cortex-M4F, 100 MHz, 512 KiB flash, 128 KiB SRAM,
LQFP64) is the same die as the STM32F411CEU6 already modelled for the WeAct
Black Pill. `configs/chips/stm32f411re.yaml` carries the same register set and
adds the LQFP64 ADC pads (PC0..PC5 = ADC1_IN10..IN15). The Nucleo-64 board has
LD2 on PA5 (Arduino D13) and USART2 on PA2/PA3 to the ST-LINK virtual COM port.

**Scope of the PR-gated smoke:** ARM reset, valid SRAM stack, RCC AHB1/APB1
enables, PA2 AF7, USART2 TX, and LED edges on PA5. The stock STM32duino
Arduino matrix row (L0–L8) is in `evidence/arduino-matrix/`. DMA1/DMA2,
USB OTG FS, SDIO, CRC, WWDG and DBGMCU are zero-answering stubs, the same as
on the Black Pill descriptor. No physical hardware comparison has been made.

## Reproduce

From the labwired-core repository root:

```sh
./examples/nucleo-f411re/firmware/build.sh
labwired test --script examples/nucleo-f411re/io-smoke.yaml \
  --output-dir out/nucleo-f411re-smoke --no-uart-stdout
```

Expected: `3/3` passing (UART contains `STM32F411RE_OK`; PA5 at least four
edges; reaches the step budget without a simulation error).

The stock Arduino row:

```sh
python3 validation/arduino-matrix/run_matrix.py --boards stm32f411re
```

The ELF in `tests/fixtures/board-smoke/stm32f411re.elf` is committed so users
with only an installed CLI can run it without an ARM compiler. Its source is
`firmware/main.c` and `firmware/linker.ld`.
See `VALIDATION.md` for the recorded evidence.
