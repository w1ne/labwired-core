# STM32U575 60North Renode-derived acceptance runbook

This runbook reproduces the **60North `renode-stm32u5`** behavioural acceptance
matrix against LabWired's STM32U575 model. It is the cross-simulator evidence
for [`docs/boards/stm32u575.md`](../../docs/boards/stm32u575.md) and the
NUCLEO-U575ZI-Q example.

> **This is a Renode-derived reference, not silicon.** Passing this suite means
> “LabWired reproduces the 60North reference model's documented behaviour”. It
> does **not** mean the model matches a bench part. There is no U575 silicon
> capture and no MMIO/silicon differential is claimed.

## Reference pin

| Item | Revision |
|------|----------|
| 60North `renode-stm32u5` | commit `9cef86b9a` (2026-10-22), Renode 1.17.0 |
| Reference worktree used here | `/home/andrii/projects/renode-stm32u5-60north` |
| STM32CubeU5 | **v1.9.0**, `d88042df24f16799957c24f00c2b234c9e306188` (pinned in `firmware-tests/cmake/st-examples.cmake`) |
| ST Nucleo BSP | `stm32u5xx-nucleo-bsp` v1.2.3, `ed50f1d2d4940e4efe23104027b8008ce79037f5` |
| ST U575I-EV BSP | `stm32u575i-ev-bsp` v1.4.1, `83d8fb77a80a4c6d458f37f6ec10471ba95ddc96` |
| ST HAL driver | `stm32u5xx-hal-driver` v1.6.3, `2552682ee5a0d6f826a6435750fc9267be65868b` |
| CMSIS device U5 | `cmsis-device-u5` v1.4.3, `624374fa1e21ca195d6f2102ac0caaa50d0ea4c8` |

The 60North platform is the reference implementation; the CubeU5 example
projects are unmodified ST sources pulled at the commit above. The firmware is
**not vendored into LabWired** — it is an external build artifact.

## What the target owns

- Test file: `crates/core/tests/stm32u575_60north.rs`
- Cargo test target: **`stm32u575_60north`**
- System under test: `configs/systems/nucleo-u575zi.yaml`
- The target is owned by the model branch (`agent/u5-60north`). It is not part
  of the package branch until that branch merges. This runbook documents how to
  reproduce it; it does not vendor the model changes or the ELFs.

## Build the 60North firmware

Requirements: Arm GNU Toolchain, CMake, Ninja, Git, Python 3 (the same
requirements as 60North's own README).

```bash
cd /home/andrii/projects/renode-stm32u5-60north
cmake -S firmware-tests -B build/firmware -G Ninja
cmake --build build/firmware
```

This produces 13 ELFs in `build/firmware/`:

```text
core-smoke.elf
clock-init.elf
hal-clock.elf
hal-gpio.elf
hal-uart.elf
hal-spi.elf
hal-spi-dma.elf
hal-spi-dma-loopback.elf
hal-uart-dma.elf
st-spi-fullduplex-polling-master.elf
st-i2c-twoboards-polling-master.elf
st-dma-flash-to-ram.elf
st-fdcan-loopback.elf
```

## Run the LabWired acceptance matrix

From the LabWired repo root, on the branch that carries the `stm32u575_60north`
target:

```bash
RENODE_U5_FIRMWARE=/home/andrii/projects/renode-stm32u5-60north/build/firmware \
  cargo test -p labwired-core --test stm32u575_60north -- --nocapture
```

`RENODE_U5_FIRMWARE` overrides the target's compiled-in default (the same
60North path on the reference machine). Missing ELFs are a hard failure, never a
skip.

### Expected result symbols per case

All 14 cases read result symbols/registers from the running image — they do not
accept “exit 0”, a banner, or a UART line as the verdict.

| # | Test case | ELF | Asserted result |
|---|-----------|-----|-----------------|
| 1 | `core_smoke_maps_memories_and_services_interrupts` | `core-smoke.elf` | `passedChecks` CHECK_DONE bit 31 and bits 0–8 set; `flashSizeKb == 2048`; `irqLogCount == 2`; `sysTickCount` still advances after 4M steps; `faultCount == 0` |
| 2 | `core_smoke_restarts_after_machine_reset` | `core-smoke.elf` | CHECK_DONE still set after `Machine::reset()` and 2M steps |
| 3 | `clock_init_reaches_160mhz_and_done_stage` | `clock-init.elf` | `failedStage == 0`, `stage == 6`; `RCC_CFGR1.SWS == PLL1`; `RCC_CR.PLL1RDY`; `FLASH_ACR.LATENCY == 4`; `PWR_VOSR.VOS == range 1`; `VOSRDY\|BOOSTRDY` |
| 4 | `hal_clock_completes_and_reports_160mhz` | `hal-clock.elf` | `failedStep == 0`, `halStatus == 0`, `step == 7`; `sysclkHz`, `hclkHz`, `pclk1Hz`, `pclk2Hz`, `pclk3Hz`, `systemCoreClockHz` all `160000000`; `HAL_Delay(10)` advances `tickAfterDelay - tickBeforeDelay == 11` |
| 5 | `hal_gpio_toggles_mirrors_and_counts_rising_edges` | `hal-gpio.elf` | `ready == 1`, `toggleOk == 1`; `mirroredLevel` and PA5 follow PB13; `risingEdges` 0→1→1→2, `lastRisingPin == 0x2000` (PC13) |
| 6 | `hal_uart_prints_configures_baud_and_echoes` | `hal-uart.elf` | `failedStep == 0`, `usartBrr == 1389`; prints `Interrupt transmit done`; RX `hello renode` echoes `HELLO RENODE`; `receivedCount == 12` |
| 7 | `hal_spi_loops_back_8_and_16_bit_frames` | `hal-spi.elf` | `failedStep == 0`, `step == 12`; `polling8Ok`, `interrupt8Ok`, `polling16Ok`, `limited8Ok` all `1`; `transferComplete == 1` |
| 8 | `hal_spi_dma_transmits_all_bytes` | `hal-spi-dma.elf` | `failedStep == 0`, `step == 6`, `transferError == 0`, `transferComplete == 1` |
| 9 | `hal_spi_dma_loopback_receives_what_it_sent` | `hal-spi-dma-loopback.elf` | `failedStep == 0`, `step == 6`, `transferError == 0`, `transferComplete == 1`, `dataOk == 1` |
| 10 | `hal_uart_dma_receives_text` | `hal-uart-dma.elf` | banner `STM32U575 HAL UART DMA ready`; `failedStep == 0`, `step == 10`, `receiveError == 0`, `receiveComplete == 1`, `dataOk == 1` |
| 11 | `st_spi_fullduplex_polling_master_sends_and_receives` | `st-spi-fullduplex-polling-master.elf` | SPI1 loopback: LED3 (PG2) off, LED2 (PB7) and LED1 (PC7) on; `aRxBuffer == aTxBuffer` for `0x73` bytes |
| 12 | `st_i2c_twoboards_master_writes_and_reads_back` | `st-i2c-twoboards-polling-master.elf` | I2C echo slave at 10-bit `0x30F` (L4 controller matches `0x07`): after press 1 LED2 on / LED3 off; after press 2 LED2 off / LED3 off; `aRxBuffer == aTxBuffer` for `0xA2` bytes |
| 13 | `st_dma_flash_to_ram_copies_and_blinks` | `st-dma-flash-to-ram.elf` | `transferCompleteDetected == 1`, `transferErrorDetected == 0`, LED3 off; `aDST_Buffer == aSRC_Const_Buffer` for 128 bytes (GPDMA1 channel 0, BNDT = 128) |
| 14 | `st_fdcan_loopback_receives_its_own_frames` | `st-fdcan-loopback.elf` | `RxData == TxData2` (12 bytes, the last FIFO1 retrieval); LED6 (PH7) off |

The clock/VOS/flash assertions in case 3 are the register-level checks: they
read `RCC_CFGR1`, `RCC_CR`, `FLASH_ACR` and `PWR_VOSR` directly after the
firmware reports `stage == 6`.

## Run the 60North Robot suites for comparison

This is optional cross-checking in Renode itself, not part of the LabWired gate:

```bash
cd /home/andrii/projects/renode-stm32u5-60north
curl -sSfL https://builds.renode.io/renode-1.17.0.linux-portable.tar.gz | tar xz
pip install -r renode_1.17.0-portable/tests/requirements.txt
renode_1.17.0-portable/renode-test --results-dir build/test-results \
  $(find tests -name '*.robot')
```

The board-level suites map to the ST example cases above:

- `tests/boards/nucleo-u575zi-q-st-examples.robot` — SPI1 loopback, I2C echo,
  GPDMA flash-to-RAM.
- `tests/boards/stm32u575i-ev-st-examples.robot` — FDCAN loopback on the
  STM32U575I-EV LED5 (PB7) / LED6 (PH7).

## Board references used by the 60North firmware

- NUCLEO-U575ZI-Q: LED1 PC7, LED2 PB7, LED3 PG2, USER button PC13, COM1 =
  USART1 PA9/PA10 — `stm32u5xx-nucleo-bsp` v1.2.3 (`stm32u5xx_nucleo.h`).
- STM32U575I-EV: LED5 PB7, LED6 PH7 active high; LED7/LED8 are on the I/O
  expander; USER button PC13; COM1 = USART1 PA9/PA10 —
  `stm32u575i-ev-bsp` v1.4.1 (`stm32u575i_eval.h`).

## Honesty boundary

- ✅ LabWired's U575 model is compared against a second, independent
  implementation of RM0456: the 60North Renode platform.
- ⚠️ Both are software models. Agreement does not prove silicon behaviour.
- ❌ No bench part, no SWD trace, no MMIO diff against a physical STM32U575 is
  part of this evidence.
