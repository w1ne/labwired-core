# NUCLEO-H563ZI Real Board Blink + UART Firmware

This firmware is for real hardware validation on the NUCLEO-H563ZI board.
It is separate from the emulator-only Rust smoke firmware and uses STM32H563 CMSIS startup/system files.

## Behavior

1. Initializes `USART3` (COM1 VCP) on `PD8/PD9` at `115200 8N1`.
2. Toggles onboard LEDs `PB0`, `PF4`, `PG4`.
3. Samples button `PC13`.
4. Prints lines such as:
   - `H563-BLINK-UART`
   - `BLINK 0 PB0=1 PF4=1 PG4=1 BTN13=0/1`

## Build

From this directory:

```bash
make clean
make
```

Output:
- `build/h563_blink_uart.elf`
- `build/h563_blink_uart.bin`

## Required Inputs

The Makefile expects STM32CubeH5 sources at:
- default: `../../../../../STM32CubeH5`
- override with: `STM32CUBE_H5_DIR=/path/to/STM32CubeH5 make`

## Source References

1. `Drivers/BSP/STM32H5xx_Nucleo/stm32h5xx_nucleo.h`
   - COM1/USART3 mapping and board pin mapping.
2. `Drivers/CMSIS/Device/ST/STM32H5xx/Include/stm32h563xx.h`
   - register definitions.
3. `Drivers/CMSIS/Device/ST/STM32H5xx/Source/Templates/system_stm32h5xx.c`
   - reset clock baseline (`HSI 64 MHz`).
4. `Drivers/CMSIS/Device/ST/STM32H5xx/Source/Templates/gcc/startup_stm32h563xx.s`
   - vector table and reset sequence.
5. `Drivers/CMSIS/Device/ST/STM32H5xx/Source/Templates/gcc/linker/STM32H563xx_FLASH.ld`
   - memory layout and linker sections.
