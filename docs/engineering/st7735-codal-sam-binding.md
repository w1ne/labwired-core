# Captured CODAL commands through SAM — qualification pending

This extends the bounded [rectangular guest](st7735-rectangular-controls.md),
not the production panel descriptor or native DMA runtime.

## Actual source capture

The original pinned driver/header executed successfully in
[trace run37623161527](https://github.com/CrispStrobe/labwired-core/actions/runs/37623161527/job/112798202082)
at harness source `f426a724fd6924d8ae74585a44e0722e8503e437`.
Three cases cover byte, word and word-plus-tail packing; the deliberately corrupt
CASET capture failed as required. This passing trace job alone does not establish
the broader PR's qualification or merge status.

`crates/core/tests/fixtures/st7735-codal-host-trace.json` preserves the original
JSON member bytes from [artifact11483103038](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11483103038/zip).
Its SHA256 is `2e4d70928807baa1b3110a2a09ef6c32d9482ab6fa32ac0ce0cd8efe76b2d062`.
It records source/header hashes, compiler, all commands and scope exclusions.
The hosted trace job requires that exact fixture hash and compares freshly
executed driver commands against it. Compiler-version metadata is retained, not
silently normalized or required to match a future host's compiler string.

## Proposed authored guest proof

The sixth test in `crates/core/tests/st7735_sam_guest.rs` embeds those captured
CASET, RASET, RGBSET and RAMWR payloads into authored Thumb instructions. The
guest configures clocks, mux and SPI, drives CS/D/C and polls TXC. It writes the
actual production palette upload, not the earlier independently authored LUT.
The host attaches only the explicit fixture crop; it neither seeds controller
completion nor writes display RAM/MMIO configuration on the guest's behalf.

Three nonzero windows (2×3, 2×4, 2×5 physical controller-memory crops) run through
ordinary and forced-legacy controller paths. Literal expected RGB888 pixels
include distinct channels, a non-extreme colour and two **known black** pixels
from zero-valued palette entries. Expectations use no codec/addressing helper.
A named negative removes one RGBSET byte from the guest's transmitted stream:
SPI must finish but all visible pixels remain unknown and frame bytes withheld.

## Acceptance and next tasks

No new guest passing result is claimed yet. Require all six guest tests to
execute and pass in feature-on and feature-off jobs, the live driver/fixture
comparison, and every enabled exact-head check. Retain exact source/guest/fixture
hashes and tested/landed tree identities before normal own-fork merge.

This executes captured driver bytes on ARM, **not the original CODAL ARM code**,
DMA driver, interrupt/fiber completion, deployed CF2/macros or actual module.
Complete independent all-orientation, odd-pixel interruption, reset/LUT retention
and panel aperture/GM evidence before closing P3. Review out-of-range window
semantics separately: the prototype currently clamps bounds, whereas the
[ST7735R datasheet](https://cdn-shop.adafruit.com/datasheets/ST7735R_V0.2.pdf)
sections 10.1.19–20 specify ignoring out-of-range data. Do not promote that
prototype policy to silicon fidelity. Glass/BGR, IRQ/DMA, active-frame RTx,
new-panel WASM rendering and consumer adoption remain unqualified.
