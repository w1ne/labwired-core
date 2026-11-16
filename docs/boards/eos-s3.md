# EOS S3 validation status

## Current measurement (2026-11-16)

The imported EOS S3 system booted `firmware-eos-s3-smoke` for **600,000
instructions** and passed **7/7 smoke assertions**. The firmware runs twelve
checks and prints one letter per check; the UART capture is
`EOS S3 checks PPPPPPPPPPPP DONE`, and the summary word at 0x2007FF00 is
0xE0530FFF. GPIO 4 shows 6 edges. Firmware SHA-256
`af5bed156746e9b8ad97554e81eda2eab3afc6000acae6946cc33d2d3718bbab`. The
result, UART capture, JUnit assertions, snapshot and build/run logs are under
[`docs/coverage/renode-onboarding/metrics/eos-s3/`](../coverage/renode-onboarding/metrics/eos-s3/).
The QuickFeather and Qomu rows run the same firmware on the same chip
surface ([`eos-s3-quickfeather.md`](eos-s3-quickfeather.md),
[`eos-s3-qomu.md`](eos-s3-qomu.md)).

Checks (source: `crates/firmware-eos-s3-smoke/src/main.rs`):

0. A constant linked at 0x0 reads back through the 0x20000000 view.
1. A `.bss` store at 0x20040000 reads back through the 0x0 view.
2. A store at 0x0003F000 reads back at 0x2003F000.
3. SDMA SRAM (0x4000F000, 4 KiB) holds a word at the first and last slot.
4. IO_OUTPUT keeps bits [7:0] only (write 0xFFFFFF50 reads 0x50).
5. IO_INPUT reads the pad inputs, not IO_OUTPUT (reads 0 with IO_OUTPUT 0xFF).
6. SW_INTR_1 with its M4 enable clear is stored and raises nothing.
7. Setting SW_INTR_1_EN_M4 over the held request enters NVIC 1 once.
8. SW_INTR_2 enters NVIC 0 once; nothing else fires.
9. A second SW_INTR_1 request enters again; the enable words keep their
   qorc-sdk widths.
10. An injected PL011 RX byte (`R`) sets OTHER_INTR bit 1 and enters NVIC 7;
    the handler's DR read clears the status and the pend.
11. SPT: with SPT_EN clear, UPDATE_TMR_VAL = 5 loads TIMER_VAL and it holds;
    with SPT_EN set it counts 5 -> 6 -> 7, and one count takes
    55,000..68,000 SysTick core cycles (measured `0xEFFF` = 61,439; nominal
    61,440 at 61.44 MHz); with SPT_EN clear again it stays at 7.
    MS_CNT_VAL reads 0x40 and EVENT_CNT_VAL the FFE_TO_PERIOD value, as in
    Renode.

## Memory map fix

The EOS S3 M4 has one 512 KiB SRAM (Renode `m4mem`), mapped at 0x20000000
and mirrored at 0x0; the bootloader copies the application into it from SPI
flash. The import mapped two separate 512 KiB stores, so a store through one
view never showed in the other. The descriptor now declares the SRAM as
`ram`, an empty `flash`, a same-backing mirror `m4mem_low` (`alias_of`) at
0x0, and `flash_boot_alias: false`. The firmware links code in the low view
of the lower half and data in the upper half (`memory.x`).

## Claimed peripherals

| id | model | scope |
|---|---|---|
| `systick` | `systick` | Part of the Renode NVIC (`systickFrequency` 61.44 MHz); the SPT rate reference. |
| `uart` | `pl011` | TX; RX with RIS/MIS and the RX level on source IRQ 7. |
| `intr_ctrl` | `eos_s3_intr_ctrl` | SW_INTR_1/2 -> NVIC 1/0; OTHER_INTR bit 1 (PL011 RX) -> NVIC 7. |
| `gpio` | `gpio`, `eos_s3` layout | MISC_CTRL IO_INPUT (0x40005100) and IO_OUTPUT (0x40005104), GPIO 0-7. |
| `spt` | `eos_s3_spt` | 30-bit 1 ms TIMER_VAL at `cpu_hz` (default 61.44 MHz), UPDATE_TMR_VAL load, SPT_CFG / SLEEP_MODE / SPARE_BITS storage. |

Memory windows: SRAM 0x20000000 (512 KiB), mirror at 0x0, SDMA SRAM
0x4000F000 (4 KiB).

Sources: Renode `platforms/cpus/eos-s3.repl`, `EOSS3_IntrCtrl.cs` and
`EOSS3_SimplePeriodicTimer.cs`; QuickLogic qorc-sdk `HAL/inc/eoss3_dev.h`
(MISC_CTRL_BASE 0x40005000, IO_INPUT +0x100, IO_OUTPUT +0x104;
`INTR_CTRL_TypeDef`; `SPT_REGS_TypeDef`, SPT_BASE 0x40005C00) and
`HAL/src/eoss3_hal_gpio.c` (`HAL_GPIO_Read`/`HAL_GPIO_Write`).

Not modelled, and stated in the descriptor:

- The IO_MUX pad controls (output enable, receive enable, pulls) are not
  modelled. IO_OUTPUT drives every pad; IO_INPUT reads only what the outside
  world drives, as in Renode.
- INTR_CTRL sources other than the two software requests and PL011 RX: no
  GPIO edge or level is detected, and no timer or watchdog is routed.
- SPT: no FFE kick-off output (no FFE model), no interrupt timestamps, no
  clock-source select, no error compensation (0x08..0x28 read 0), no rate
  change when firmware re-clocks the M4.

Not declared (no model in this engine): IO_MUX, ADC, DesignWare SPI and SPI
DMA, packet FIFO, uDMA and its bridge, FFE and its I2C masters, voice block,
PMU, CRU.

## Negative controls

- [`attempt-history/eos-s3/2026-11-09-negative-control-old-descriptor/`](../coverage/renode-onboarding/attempt-history/eos-s3/2026-11-09-negative-control-old-descriptor/):
  the old descriptor; BusFault at the SDMA SRAM (check 3), no UART output.
- [`attempt-history/eos-s3/2026-11-09-negative-control-split-stores/`](../coverage/renode-onboarding/attempt-history/eos-s3/2026-11-09-negative-control-split-stores/):
  this descriptor with the two separate stores; `FFFPPP`, checks 0-2 fail.
- [`attempt-history/eos-s3/2026-11-09-negative-control-no-gpio/`](../coverage/renode-onboarding/attempt-history/eos-s3/2026-11-09-negative-control-no-gpio/):
  without the GPIO port; BusFault at IO_OUTPUT (check 4).

## Superseded measurements

- PL011 banner only, against the two-store map: replaced by this
  measurement; the old files are in git history before this change.

The profile's `verified` field remains false. This result is limited to the
local model behaviour; it is not datasheet validation or hardware
confirmation.
- [`attempt-history/eos-s3/2026-11-15-negative-control-head-descriptor/`](../coverage/renode-onboarding/attempt-history/eos-s3/2026-11-15-negative-control-head-descriptor/):
  without INTR_CTRL; BusFault at 0x40004840 (check 6).
- [`attempt-history/eos-s3/2026-11-16-negative-control-no-spt/`](../coverage/renode-onboarding/attempt-history/eos-s3/2026-11-16-negative-control-no-spt/):
  without the SPT window; BusFault at 0x40005C00 (check 11), FAIL 1/7.
- [`attempt-history/eos-s3/2026-11-16-negative-control-spt-half-clock/`](../coverage/renode-onboarding/attempt-history/eos-s3/2026-11-16-negative-control-spt-half-clock/):
  SPT at `clock_hz` 30,720,000; one count takes `0x7800` cycles, check 11
  fails, FAIL 5/7.
