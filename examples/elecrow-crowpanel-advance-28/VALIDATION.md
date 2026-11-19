# CrowPanel Advance 2.8 V1.2 — local digital-twin validation

Tested: **2026-11-10**, isolated worktree based on `a3c73a949` (origin/main).

## Artifacts and toolchain

- Arduino-ESP32 via PlatformIO 6.2.0 (`esp32-s3-devkitc-1`, no extra libraries).
- Generated `firmware.elf`, `firmware.bin`, `bootloader.bin`, `partitions.bin`, merged `flash.bin` using esptool 5.4.0. The generated flash image is local, not committed.
- LabWired CLI compiled locally from this repository's release profile (v0.25.0).
- Runner: `LABWIRED_BIN=/path/to/labwired bash examples/elecrow-crowpanel-advance-28/verify.sh`.
- Built source: `src/main.cpp`; virtual hardware: `system.yaml`.

## Observed outcomes

| Check | Result | Evidence |
|---|---|---|
| Compile ESP32-S3 Arduino sketch | **PASS** | PlatformIO reports RAM 22,224 B / 327,680 B, flash 332,814 B / 6,553,600 B. |
| ROM-boot + UART | **PASS** | `CROWPANEL_DISPLAY_TRANSFER_OK` was reached after the real firmware's ST7789 SPI writes. |
| Display shape | **PASS** | 320×240 PPM (MADCTL.MV landscape). |
| RGB display pixels | **PASS** | Red (255,0,0): 2,304; green (0,255,0): 2,304; blue (0,0,255): 2,304; black: 69,888. |
| Negative control: wrong LCD D/C wire | **PASS** | Same compiled firmware and MCU, `dc_pin` changed from GPIO41 to GPIO4; PPM black: 76,800 pixels, zero painted color. |
| Physical CrowPanel verification | **NOT DONE** | No physical DIS01728A V1.2 attached to this test. |
| Touch / power / RF / audio | **NOT TESTED** | Intentionally excluded from the first milestone. |

The oracle inspects the simulated pixels, not only UART text. The negative control demonstrates that the display depends on correct D/C wiring. This is a *functional local simulation* of the selected digital path, **not** a blanket silicon-fidelity or production-ready claim.

Known model limits: ST7789 controller emulated with reused `st7789-170x320` generic descriptor (full 240×320 RAM with no crop), no GPIO-level RES or BLK electrical validation. Color inversion/landscape must be checked on real hardware.

## Next step with Elecrow

Ask their R&D team to run the same sketch on DIS01728A V1.2, compare the visible red/green/blue rectangles and UART markers, and sign off the exact board revision and pin map. Only after that consider a public Playground/gallery catalog entry.
