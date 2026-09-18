# External Components (NUCLEO-U575ZI-Q)

No required external simulated components: this is a bare-board onboarding and
the deterministic smoke tests use on-chip peripherals only.

1. RCC (Reset and Clock Control) — U5 PLL1 bring-up + ready bits
2. PWR — voltage scaling / supply selection
3. FLASH interface — ACR latency (program/erase not modeled)
4. GPIO — LD1 PC7, LD2 PB7, LD3 PG2, USER PC13
5. USART1 — Virtual COM port over PA9/PA10
6. SysTick + NVIC
7. CRS — register surface used by the Arduino core's `SystemClock_Config`

The fidelity matrices attach a **declarative** kit (a model, not a physical
component requirement):

- `validation/arduino-matrix/systems/stm32u575.yaml` — INA219 at `0x40` on
  `i2c1`, MAX31855 on `spi1` (CS `PA4`)
- `validation/zephyr-matrix/systems/stm32u575.yaml` — INA219 at `0x40` on `i2c1`

## CubeU5 checkout (external, not committed)

The vendor HAL firmware links against a stock STM32CubeU5 checkout that the
Makefile expects as a **sibling of this repository** (its default is
`../../../../STM32CubeU5`, resolved from `examples/nucleo-u575zi/board_firmware`):

```bash
git clone --depth 1 https://github.com/STMicroelectronics/STM32CubeU5 ../STM32CubeU5
git -C ../STM32CubeU5 rev-parse HEAD
# 12d19a5358da129dc74aecff1adee370218ca186  (recorded 2026-09-17)
```

Submodule revisions used by the HAL build (initialized by `--depth 1` clone):

- HAL driver `0e5fefb8dc2d6afa60816ebbf8b1672cfec4595b`
- CMSIS device `624374fa1e21ca195d6f2102ac0caaa50d0ea4c8`

Build (override the checkout location with `STM32CUBE_U5_DIR=/path`):

```bash
make -C examples/nucleo-u575zi/board_firmware
```

The BSP submodule is **not** needed: the firmware drives the HAL directly
(no `stm32u5xx_nucleo.c`), so only the HAL driver + CMSIS device submodules
must be present.
