# Vendored ESP32-C6 boot-ROM image

Flat mask-ROM image for the `esp32c6` chip's `rom` region, so stock
Arduino-ESP32 3.x / ESP-IDF 5 firmware can call ROM routines with no ESP
toolchain installed.

| file | window | size |
|---|---|---|
| `esp32c6_rom.bin` | ROM `0x4000_0000..0x4005_0000` | 320 KiB |

## Provenance

Extracted from Espressif's published `esp32c6_rev0_rom.elf`
([espressif/esp-rom-elfs](https://github.com/espressif/esp-rom-elfs) release
`20260528`, ELF sha256 `788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551`)
by `scripts/make_esp32c6_rom_bin.py`. The image sha256 is
`1df445413a86710693e6c1546d8d36b4f62a7d97f1aacfffa92a9536c7b28f50`.

The script places the PT_LOAD file bytes, then overlays the PROGBITS sections
in the window that no PT_LOAD carries. That second pass matters:
`.rodata.interface` (0x4004_FD40) holds `ets_rom_layout_p` (0x4004_FFFC), and
ESP-IDF 5 heap init aborts if it reads back as zero.

The ROM contents are the ESP32-C6 mask ROM, copyright Espressif Systems
(Shanghai) Co., Ltd. They are included here solely so the simulator can
execute the chip's genuine ROM routines; they are not covered by this
repository's MIT license.

## Resolution at runtime

1. `LABWIRED_ESP32C6_ROM` (explicit flat bin; an empty value opts out)
2. This in-tree file (`bus::from_config::default_region_image_path`)

The ROM's own reset path is not run: fast boot jumps straight to the app, so
ROM `.data` is not unpacked into HP SRAM. `boot::esp32c6_rom::seed_rom_data`
seeds the one pointer the stock Arduino path needs (`heap_tlsf_table_ptr`).
