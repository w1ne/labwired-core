# nRF54L15 FICR serves the SVD register file (2026-11-27)

## Defect

`configs/chips/nrf54l15.yaml` mapped FICR (0x00FFC000) to
`nrf54l_ficr_stub`, which answered every read with 0. No nRF54L15 part
reads 0 there. Nordic's `nrf54l15.svd` gives every FICR word the reset
value 0xFFFFFFFF ("Unspecified") and enumerates INFO.PART = N54L15
(0x00054B15), INFO.RAM = K256 (0x100) and INFO.RRAM = K1524 (0x5F4).
Firmware that identifies the part (`INFO.PART`) or sizes memory from FICR
read a value that no silicon returns.

## Fix

- New descriptor `configs/peripherals/nrf54l15/ficr.yaml`: the register list,
  offsets, access and reset values of the SVD-generated
  `global_ficr_ns.yaml`, with the three INFO words set to the SVD enum
  values above. All other words keep the SVD reset value 0xFFFFFFFF.
- The chip entry uses the generic `declarative` model. No new model type.
- `nrf54l_ficr_stub` removed from `known_stubs.rs`.

Not invented: DEVICEID, DEVICEADDR, ER/IR, VARIANT, PACKAGE, XOSC trims and
the TRIMCNF table read erased. nrfx `SystemInit` (`system_nrf54l.c`) stops
its TRIMCNF copy loop when ADDR reads 0xFFFFFFFF or 0, so the boot path
copies no trim.

## Evidence

Gate `crates/core/tests/nrf/nrf54l15_ficr.rs` (3 tests) reads the enum
values from the vendored SVD text and the memory sizes from the chip
descriptor:

- INFO.PART, INFO.RAM, INFO.RRAM equal the SVD enum values.
- INFO.RAM x 1 KiB = chip RAM (256 KiB); INFO.RRAM x 1 KiB = chip RRAM
  (1524 KiB).
- DEVICEID[0..1], VARIANT and TRIMCNF[0].ADDR read 0xFFFFFFFF.
- A write to INFO.PART does not change it.

Results on `lane/renode-round772-20261127`:

- `--test nrf nrf54l`: 31/31 pass (includes the 3 new tests and the
  existing smart-ring I2C probe).
- `--test gates nrf54l15`: 3/3 pass (Zephyr v4.4 hello_world survival,
  bare smoke survival, DK LED0). Baseline before the change: 3/3.
- Negative control (descriptor pointed at the raw SVD dump): the INFO test
  fails. Log: `negative-control.log`.

## Limits

- VARIANT and PACKAGE have no SVD enum, so they read 0xFFFFFFFF, not the
  value of a specific production lot.
- No per-device DEVICEID or BLE DEVICEADDR. Zephyr `hwinfo` returns the
  erased ID.
- UICR, TAMPC, RRAMC, REGULATORS and DPPIC20/30 remain zero-filled stubs.
