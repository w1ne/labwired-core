# EOS S3 validation status

## Current measurement (2026-11-09)

The imported EOS S3 system booted `firmware-eos-s3-smoke` for **100,000
instructions** and passed **6/6 smoke assertions**. The firmware runs six
checks and prints one letter per check; the UART capture is
`EOS S3 checks PPPPPP DONE`, and the summary word at 0x2007FF00 is
0xE053003F. GPIO 4 shows 6 edges. The result, UART capture, JUnit
assertions, snapshot and build/run logs are under
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
| `uart` | `pl011` | Polled TX. |
| `gpio` | `gpio`, `eos_s3` layout | MISC_CTRL IO_INPUT (0x40005100) and IO_OUTPUT (0x40005104), GPIO 0-7. |

Memory windows: SRAM 0x20000000 (512 KiB), mirror at 0x0, SDMA SRAM
0x4000F000 (4 KiB).

Sources: Renode `platforms/cpus/eos-s3.repl` and `EOSS3_IntrCtrl.cs` (misc
region); QuickLogic qorc-sdk `HAL/inc/eoss3_dev.h` (MISC_CTRL_BASE
0x40005000, IO_INPUT +0x100, IO_OUTPUT +0x104) and
`HAL/src/eoss3_hal_gpio.c` (`HAL_GPIO_Read`/`HAL_GPIO_Write`).

Not modelled, and stated in the descriptor:

- The IO_MUX pad controls (output enable, receive enable, pulls) are not
  modelled. IO_OUTPUT drives every pad; IO_INPUT reads only what the outside
  world drives, as in Renode.
- The PL011 interrupt and the GPIO interrupts go through the EOS S3
  interrupt controller (0x40004800), which has no model.

Not declared (no model in this engine): interrupt controller, IO_MUX,
simple periodic timer, ADC, DesignWare SPI and SPI DMA, packet FIFO, uDMA
and its bridge, FFE and its I2C masters, voice block, PMU, CRU.

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
