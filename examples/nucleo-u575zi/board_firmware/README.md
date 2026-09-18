# NUCLEO-U575ZI-Q STM32CubeU5 HAL Smoke Firmware

Minimal real-toolchain firmware for the LabWired NUCLEO-U575ZI-Q example: it
uses the STM32CubeU5 HAL (no BSP, no CubeMX-generated project) to bring up the
clock tree, USART1 VCP and LD1, so the simulator can be exercised against a
genuine Cube HAL register flow.

Behavior:

1. `HAL_Init()` (SysTick 1 ms tick), ICACHE 1-way enable, MSI 4 MHz -> PLL1
   160 MHz (`FLASH_LATENCY_4`), SMPS supply, VOS scale 1.
2. `USART1` (PA9/PA10) at `115200 8N1` prints `U575-HAL OK`.
3. `PC7` (LD1) toggles every `HAL_Delay(250)` and prints
   `BLINK <n> LD1=<0|1>`.

## Build

Requires `arm-none-eabi-gcc` and an STM32CubeU5 checkout at
`../../../../STM32CubeU5` (sibling of the repo root), with the
`Drivers/STM32U5xx_HAL_Driver` and `Drivers/CMSIS/Device/ST/STM32U5xx`
submodules initialized. Point the build at a different checkout with:

```bash
make STM32CUBE_U5_DIR=/path/to/STM32CubeU5
```

Output: `build/u575_hal_smoke.elf`.

CubeU5 revision used: `12d19a5358da129dc74aecff1adee370218ca186`
(HAL driver submodule `0e5fefb8dc2d6afa60816ebbf8b1672cfec4595b`,
CMSIS device submodule `624374fa1e21ca195d6f2102ac0caaa50d0ea4c8`).

## Run in LabWired

```bash
cargo run -q -p labwired-cli -- \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000
```

`20000000` steps is about 0.14 s of simulated time, so the expected output is
exactly:

```
U575-HAL OK
BLINK 0 LD1=1
```

Each blink costs 250 ms of simulated time (~36M steps at 160 MHz); raise the
budget to see later lines. With `--max-steps 50000000`:

```
U575-HAL OK
BLINK 0 LD1=1
BLINK 1 LD1=0
```

For determinism diffs, capture **stdout only** — stderr carries progress and
timing logs whose IPS varies run to run. `RUST_LOG=off` suppresses them
entirely, leaving the UART stream alone on stdout.

The vendored `stm32u5xx_hal_conf.h` is copied from
`Projects/NUCLEO-U575ZI-Q/Templates/TrustZoneDisabled/Inc/` with the module
list trimmed to HAL core, RCC(+EX), PWR(+EX), FLASH(+EX), GPIO, UART, CORTEX
and ICACHE, and `HSE_VALUE` set to 8 MHz.
