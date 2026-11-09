# ESP32: stock `LittleFS` keeps files larger than one block; `__builtin_popcount` works

**Demand rank:** ESP32 is #1 (38 % of respondents, JetBrains State of Embedded
2025). This is depth work on a part that is already modelled, not a new part.
`LittleFS` is the standard file system in arduino-esp32 3.x (config files,
web assets, logs). It follows the `Preferences`/NVS fix in
`docs/receipts/2026-11-09-esp32-preferences-nvs/`.

**Vendor status:** not re-read this round. The NVS receipt of the same day
records espressif.com/en/products/longevity-commitment: ESP32 "At Least 15
Years" from 2016, ESP32-S3 "At Least 12 Years" from 2021, neither NRND.

## What was wrong

On the classic ESP32 and the ESP32-S3, mask-ROM functions are thunks in a
`RomThunkBank`. An address with no registered thunk falls back to
`nop_return_zero`. The libgcc bit-operation helpers were not registered:

| symbol | ESP32 | ESP32-S3 | who calls it |
|---|---|---|---|
| `__popcountsi2` | 0x40002ed0 | 0x400024d8 | `__builtin_popcount`; 8 objects in the stock ESP32 libs: littlefs `lfs.c`, eFuse `esp_efuse_fields.c` and `esp_efuse_utility.c`, I2S PDM, DAC continuous, BT, BLE mesh, Wi-Fi mesh |
| `__popcountdi2` | 0x40002ef8 | 0x400024cc | `__builtin_popcountll` |
| `__paritysi2` | 0x40002f3c | 0x400024c0 | `__builtin_parity` |
| `__ctzdi2` | 0x4000ca64 | 0x4000222c | 3 objects in the stock libs |
| `__ffsdi2` | 0x4000ca2c | 0x400022b0 | 1 object in the stock libs |
| `__clzdi2` | 0x4000ca50 | 0x40002208 | libgcc family, same gap |
| `__ffssi2` | 0x4000c804 | 0x400022bc | libgcc family, same gap |

Each returned 0 for every input. littlefs uses `popc(index)` to find where
the data starts in each block of a file (after the CTZ skip-list pointers).
With `popc(1) = 0`, the read side looked 4 bytes too far into block 1. The
write side did not depend on the error. Result: a file of 4096 bytes or less
read back correctly. Any larger file read back shifted (8000 B: 3904 bad
bytes) or failed with `EBADMSG` (8 KiB and larger).

Addresses are from `esp32.rom.libgcc.ld` / `esp32s3.rom.libgcc.ld` in
`framework-arduinoespressif32-*-libs` 3.3.12.

## How it was found

1. `sketch-littlefs` failed on the ESP32: the 24 KiB file read 0 bytes.
2. `sketch-size-probe`: 4000 and 4096 bytes pass, every file that spans a
   second block fails. A raw `esp_partition_read` showed the flash content
   was correct, and unaligned flash reads were correct.
3. The bad data equals the flash content 4 bytes further on. In
   `lfs_ctz_index` that happens only when `popc(1)` returns 0.
4. `sketch-bitops` confirmed it: `popcount=0 popcountll=0 parity=0`.

## Proof

Firmware: stock arduino-esp32 3.3.12 (PlatformIO `espressif32@8.0.0`), for
`esp32dev` and `esp32-s3-devkitc-1`. Systems:
`validation/arduino-matrix/systems/{esp32,esp32s3}.yaml`. "before" is the lane
CLI at 325663bfd. "after" is the same tree with this change. Both runs used
the same ELFs.

| check | before | after |
|---|---|---|
| ESP32 `sketch-littlefs` (format, write, append, rename, 24 KiB write + read-back, list, remove, remount) | `read=0`, `LW_FS_FAIL` | `read=24576 bad=0`, `LW_FS_OK` |
| ESP32 `sketch-size-probe` 8000 B | `bad=3904` | `bad=0` |
| ESP32 `sketch-bitops` | `popcount=0 popcountll=0 parity=0`, FAIL | `9 15 1`, `LW_BITOPS_OK` |
| ESP32-S3 `sketch-bitops` | `popcount=0 popcountll=0 parity=0`, FAIL | `9 15 1`, `LW_BITOPS_OK` |
| ESP32-S3 `sketch-littlefs` | boot abort, no UART | boot abort, no UART (not fixed, see below) |

Regression check with the after binary: Arduino matrix `esp32` row 9/9 pass.
`esp32s3` row 8/9: L4 fails with `frame=0x01010101` on the before binary
too, so this change did not cause it.

## Not covered

- **ESP32-S3 LittleFS.** The larger LittleFS image aborts in
  `system_early_init` with "Running on single core variant of a chip, but app
  is built with multi-core support". That is in the S3 fast-boot harness, not
  in the file system. Open.
- **ESP32-C3 / C6 LittleFS.** RISC-V parts with real ROM images, so they do
  not have this bug. On the lane, the C3 matrix L0 fails to boot (separate
  issue). The C6 reports `g_rom_flashchip.page_size = 0` from
  `LittleFS.begin()`. Both are open.
- The other unregistered ROM libgcc helpers (`__muldi3`, `__divsi3`, the
  overflow-trapping `*v*` family, `__cmpdi2`, ...). Stock libs and the
  probe sketch do not call them (GCC inlines them on the LX6/LX7), so they
  were left alone.
- Persistence across a power cycle. Each run starts from erased flash.
