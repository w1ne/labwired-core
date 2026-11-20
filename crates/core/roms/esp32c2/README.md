# Vendored ESP32-C2 boot-ROM image

Flat mask-ROM image for the `esp32c2` chip's `rom` region, so esp-hal and
other bare-ELF firmware can call ROM routines (memcpy, the `rom_i2c_*`
regi2c helpers, cache calls) with no ESP toolchain installed.

| file | window | size |
|---|---|---|
| `esp32c2_rom.bin` | IROM `0x4000_0000..0x4009_0000` | 576 KiB |

The DROM window `0x3FF0_0000..0x3FF5_0000` is the same ROM; the descriptor
maps it as an `alias_of: 0x40040000` mirror, so there is no second file.

## Provenance

Extracted from Espressif's published `esp32c2_rev200_rom.elf` (chip revision
v2.x, the ESP8684H4X)
([espressif/esp-rom-elfs](https://github.com/espressif/esp-rom-elfs) release
`20260528`, tarball sha256
`caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46` as
published in the release checksum file, ELF sha256
`289ce00e79c024d9efeb29a7fbc2d21853a1cbc383a2b9c2ece85c54fa829dd9`) by
`scripts/make_esp32c2_rom_bin.py`. The image sha256 is
`f1b6f84ae7a8fbd5428c85f2296b69544e1d443908bdb2356a8c6d5722c5c48d`.

The script places each PT_LOAD at its load (physical) address. That keeps
the `.data_*` initialisers (LMA `0x4008_23B0..`) and `.rodata` (LMA
`0x4008_2C00`, read through the DROM alias at `0x3FF4_2C00`) in the image.
Fast boot replays the ROM's own `_rom_store_table` copy from these bytes
(`crates/core/src/boot/esp32c2_rom.rs`).

The ROM contents are the ESP32-C2 mask ROM, copyright Espressif Systems
(Shanghai) Co., Ltd. They are included here solely so the simulator can
execute the chip's genuine ROM routines; they are not covered by this
repository's MIT license.

## Resolution at runtime

`LABWIRED_ESP32C2_ROM` names a replacement file. Without it, the bus loads
this file through `default_region_image_path` (`crates/core/src/bus/from_config.rs`).
