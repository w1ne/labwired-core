# NUCLEO-F446RE — executable minimal model

The STM32F446RE (Arm Cortex-M4F) is modeled in LabWired using the actual ST
memory addresses and IRQs, and reused STM32F4 peripheral behavior. The board
contains LD2 on PA5, B1 on PC13 and USART2 on PA2/PA3 connected to ST-LINK VCP.

**Scope of this fixture:** ARM reset, valid SRAM stack, RCC AHB1/APB1 enables,
GPIOA AF7 configuration, USART2 TX, and observable LED GPIO edges. The timer,
PWR and flash controller addresses are available but *not* verified by this
fixture. The broader chip model includes DMA streams and additional peripherals;
the earlier stock STM32duino matrix evidence is in `evidence/arduino-matrix/`.
USB OTG, FMC, QuadSPI, DCMI and other F446 blocks remain unmodeled. No physical
hardware comparison has been made.

## Reproduce

From the labwired-core repository root:

```sh
./examples/nucleo-f446re/firmware/build.sh
labwired test --script examples/nucleo-f446re/io-smoke.yaml \
  --output-dir out/nucleo-f446re-smoke --no-uart-stdout
```

Expected: `3/3` passing (UART contains `STM32F446RE_OK`; PA5 at least four
edges; reaches step budget without a simulation error).

The ELF in `tests/fixtures/board-smoke/stm32f446re.elf` is committed so users
with only an installed CLI can run it without an ARM compiler. Its checked-in
source is `firmware/main.c` and `firmware/linker.ld`.
See `VALIDATION.md` for exact recorded evidence.
