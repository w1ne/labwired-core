# Required Source Documents (NUCLEO-U575ZI-Q)

## MCU (reference manual, datasheet, SVD)

1. STM32U575/585 reference manual **RM0456**:
   https://www.st.com/resource/en/reference_manual/rm0456-stm32u5-series-armbased-32bit-mcus-stmicroelectronics.pdf
2. STM32U575ZI datasheet **DS13737** (2 MiB flash, 768 KiB SRAM + 16 KiB SRAM4,
   160 MHz max):
   https://www.st.com/en/microcontrollers-microprocessors/stm32u575zi.html
3. Vendored CMSIS-SVD register database (bases, IRQs, reset values) —
   `tests/fixtures/real_world/stm32u575.svd`,
   sha256 `2e45c70a3387a46a07092b145301e7bee3e95480377e67e252db6bf4ecfd701a`:
   https://raw.githubusercontent.com/cmsis-svd/cmsis-svd-data/main/data/STMicro/STM32U575.svd
4. STM32U5 CMSIS device header (chip header used by the HAL build):
   https://github.com/STMicroelectronics/cmsis-device-u5/blob/main/Include/stm32u575xx.h

## Board (NUCLEO-U575ZI-Q)

1. Board user manual **UM2861** (MB1549 Nucleo-144: B1/PC13 polarity, LD1–LD3,
   USART1 VCP, Arduino connector pinout including D10/PD14 chip select):
   https://www.st.com/resource/en/user_manual/um2861-stm32u5-nucleo144-board-mb1549-stmicroelectronics.pdf
2. STM32U5xx Nucleo BSP (`COM1` = USART1 PA9/PA10, LD1/LD2/LD3):
   https://github.com/STMicroelectronics/stm32u5xx-nucleo-bsp
3. Zephyr board documentation `nucleo_u575zi_q` (console = USART1,
   `led0` = blue LD2, clock tree):
   https://docs.zephyrproject.org/3.7.0/boards/st/nucleo_u575zi_q/doc/index.html

## Optional simulated display (Star Catcher)

1. Solomon Systech's [OLED display product catalog](https://www.solomon-systech.com/product-category/oled-display)
   lists the SSD1306 as a 128 × 64 monochrome OLED driver. The Playground game
   attaches a simulated SSD1306 at I2C address `0x3C`; it does not require or
   validate a physical display module. See `EXTERNAL_COMPONENTS.md` for the
   declared wiring and simulation boundary.

## External executable reference

The MIT-licensed [60North STM32U5 Renode package](https://github.com/60north/renode-stm32u5/tree/9cef86b9a9d880c0fae0c7b6c92ff039ec8a0c82)
is pinned to `9cef86b9a9d880c0fae0c7b6c92ff039ec8a0c82`. Its platform,
peripheral models, Robot assertions and 13 firmware ELFs provide an additional
behavioral reference. The chip register contract remains grounded in the ST
sources above. This package is an external Renode model, not a silicon oracle.
Running its firmware in LabWired does not execute Renode and is not a
Renode-versus-LabWired differential or physical-board acceptance.

The `stm32u575-reference` job in `.github/workflows/core-onboarding-smoke.yml`
runs nightly, on relevant main changes, and on manual dispatch. It checks out
that exact commit, builds all 13 ELFs from source with ARM GCC/newlib, CMake and
Ninja, and runs the ignored firmware acceptance tests, register tests and
U545/U575 drift guard. It preserves configure/build/test logs, firmware SHA-256
hashes and source/toolchain revisions in the `stm32u575-60north-reference`
artifact. Any build, missing ELF or test failure fails the job.

The reference's CMake verifies the following dependency commits after fetching:

| Source | Revision | License |
| --- | --- | --- |
| STM32U5 HAL v1.6.3 | `2552682ee5a0d6f826a6435750fc9267be65868b` | BSD-3-Clause |
| CMSIS device U5 v1.4.3 | `624374fa1e21ca195d6f2102ac0caaa50d0ea4c8` | BSD-3-Clause |
| CMSIS Core v5.9.0_20250520 | `b7487de1303e2eb77c402432e368691e9dcfb5f0` | Apache-2.0 |
| STM32CubeU5 v1.9.0 examples | `d88042df24f16799957c24f00c2b234c9e306188` | BSD-3-Clause |
| STM32U5xx Nucleo BSP v1.2.3 | `ed50f1d2d4940e4efe23104027b8008ce79037f5` | BSD-3-Clause |
| STM32U575I-EV BSP v1.4.1 | `83d8fb77a80a4c6d458f37f6ec10471ba95ddc96` | BSD-3-Clause |

For local reproduction with those tools installed:

```sh
git clone https://github.com/60north/renode-stm32u5.git .external/renode-stm32u5
git -C .external/renode-stm32u5 checkout 9cef86b9a9d880c0fae0c7b6c92ff039ec8a0c82
cmake -S .external/renode-stm32u5/firmware-tests -B .external/renode-stm32u5/build/firmware -G Ninja -DSTM32CUBE_FIRMWARE=ON
cmake --build .external/renode-stm32u5/build/firmware --parallel 2
export RENODE_U5_FIRMWARE="$PWD/.external/renode-stm32u5/build/firmware"
cargo test --locked -p labwired-core --test stm32u575_60north -- --ignored --nocapture
cargo test --locked -p labwired-core --test stm32u575_60north_registers --test u545_u575_drift_guard
```

Configure requires network access for the pinned ST dependencies. Firmware
acceptance is ignored in ordinary `cargo test` runs because its external source
and ARM toolchain are intentional prerequisites; the CI job supplies both.
