# Evidence — 2026-11-10

1. **Original Elecrow source**: `example/V1.1_and_V1.2/Arduino_Code/lesson-03/2.4_2.8_LVGL` from [Elecrow-RD repository](https://github.com/Elecrow-RD/CrowPanel-Advance-2.8-HMI-ESP32-S3-AI-Powered-IPS-Touch-Screen-320x240). Compiled **successfully** locally using LovyanGFX, LVGL and GT911 Arduino library. LabWired native ESP32-S3 + ST7789 model rendered the 320×240 lamp with ON and OFF buttons (see `images/official-elecrow-lvgl-sim.png`). Simulator hit a 200M-step cap; touch behavior was not validated, and `Setup done` was not observed. **Not a full vendor application pass.**
2. **Original HMI adaptation in this repo**: `src/main.cpp` compiled with PlatformIO/Arduino (success; RAM 22,232 bytes, flash 334,102 bytes). Native simulator ROM-booted the firmware and printed `CROWPANEL_HMI_READY` and painted a 320×240 framebuffer (`images/onoff-hmi-off-sim.png`). Also reached the 200M-step budget; the SPI/UI behavior is evidenced by pixels, not a clean stop verdict.
3. **Hosted/browser**: `https://app.labwired.com/?share=a7wA8ut7SHGb&run=1` compiled and loaded the exact HMI adaptation into the browser simulator; Playwright launched signed-out Chrome and tested live inputs. Initial preview framebuffer had **320×170** dimensions due to the legacy web decoder cropping the 240-column GRAM as if it were a narrower 170×320 display.
4. **Functional ON/OFF input test** (Playwright pressed `Tap` for 12 s against each test-only button): initial preview had 0 yellow pixels and 12,146 gray pixels; ON gave **4,666 yellow and 7,480 gray**; OFF returned to **0 yellow and 12,146 gray**. No browser JS page errors in these tests. These tests verify the *test input proxy* and generated display frames, not the CrowPanel's real touchscreen.

## Gaps to close before official Elecrow launch

- Introduce a generic 240×320 ST7789 screen mode (or display artifact dimensions) in the browser, without hard-coded cropping of the 170×320 module. Test portrait/landscape orientation and do not regress the 1.9-inch part.
- Resolve Elecrow V1.2 touchscreen model. Official LovyanGFX header uses FT5x06 (I²C 0x38), while the lesson's touch.h selects GT911 (0x5D). Confirm actual V1.2 chip and wiring with Elecrow R&D before implementing either.
- Bridge touch coordinates from browser display interactions to the verified I²C touch controller. Remove the two virtual GPIO button proxies afterward.
- Verify the exact firmware and HMI transitions on one physical DIS01728A V1.2 board. Define UART and pixel acceptance captures against hardware.
- Use genuine Elecrow board visuals and proper board catalog id, not a generic Espressif DevKitC carrier.
