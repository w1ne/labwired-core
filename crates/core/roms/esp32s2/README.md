# Vendored ESP32-S2 boot-ROM images

Flat mask-ROM images for the ESP32-S2 Arduino/ESP-IDF profile
(`configs/chips/esp32s2-arduino.yaml`), so LabWired runs unmodified ESP32-S2
applications with no ESP toolchain installed.

| file | window | size |
|---|---|---|
| `esp32s2_rom.bin` | IROM (instruction bus) `0x4000_0000..0x4002_0000` | 128 KiB |
| `esp32s2_drom.bin` | DROM (data bus) `0x3FFA_0000..0x3FFB_0000` | 64 KiB |
| `esp32s2_rom_data.bin` | ROM `.data_*` in SRAM `0x3FFF_E000..0x4000_0000` | 8 KiB |

The third image holds the bytes the ROM's reset handler copies from IROM into
SRAM. LabWired enters the app at its ELF entry and skips that reset, so it
places the copy's result directly.

## Provenance

Extracted from Espressif's published `esp32s2_rev0_rom.elf` (sha256
`cabae5c901fe816aea5e74fbb621ad6479ad1b399d6e7212295191559654572d`), from
[espressif/esp-rom-elfs](https://github.com/espressif/esp-rom-elfs) release
`20260528` (checksum file verified), by `scripts/make_esp32s2_rom_bins.py`:

| file | sha256 |
|---|---|
| `esp32s2_rom.bin` | `15299c270c7ff33a94d6f9a4f2d4aeb00fda445aba62a8889b88dd87d30dc013` |
| `esp32s2_drom.bin` | `204209204d378c79e21e1df3bddf8c1bb9393c72d2d5bcc88ead29a5a7d0787f` |
| `esp32s2_rom_data.bin` | `4c4ea523bdd809253203224f2f7ecdcafef8ac941d9fa4b55a26f275491036a1` |

The ROM contents are the ESP32-S2 mask ROM, copyright Espressif Systems
(Shanghai) Co., Ltd. They are included here solely so the simulator can
execute the chip's genuine ROM code; they are not covered by this
repository's MIT license.

## Resolution order at runtime

1. `LABWIRED_ESP32S2_ROM` / `LABWIRED_ESP32S2_DROM` / `LABWIRED_ESP32S2_ROM_DATA`
2. These vendored images (`bus::from_config::default_region_image_path`)
