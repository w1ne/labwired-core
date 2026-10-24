# ST7735 retained RAM and LUT transitions — guest qualification pending

This test-only P3 slice follows the
[orientation guest](st7735-orientation-controls.md) and
[captured CODAL commands](st7735-codal-sam-binding.md). It changes no model,
production descriptor, app pin, baseline, floor, capture or acknowledgement.
No passing guest result is claimed until the complete eight-test SAM target
executes in both feature configurations and all enabled exact-head checks pass.

## Authored staged guest

The unchanged width-three CODAL capture initially paints six distinct colours
in physical columns 11–12 and rows 7–9. Its palette and RAMWR bytes remain the
original host-driver capture; the later protocol transitions are authored
Thumb instructions, not original CODAL/PXT ARM execution.

Ten guest-written RAM markers bound the following observations. The host steps
the CPU to each marker and inspects the fixed physical crop; it injects no
panel MMIO, LUT or pixels.

1. Original capture is complete and all six colours are known.
2. Changing MADCTL to MX leaves the fixed physical RAM crop unchanged.
3. Replacing all 128 LUT entries leaves previously stored colours unchanged.
4. A new RGB444 pixel changes only physical (11,7), to literal expanded
   RGB666 colour `[69,117,174]`; the other five original colours remain.
5. Software reset retains RAM, RGB444 depth and MX.
6. A new pixel at physical (12,7) proves the replacement LUT survived software
   reset and MX still controls the new write.
7. GPIO hardware reset is asserted and released with SPI gated. RAM remains
   known and unchanged, while depth returns to RGB666 and MADCTL to zero.
8. Selecting RGB444 and writing physical (11,8) without reinstalling a LUT
   makes exactly one pixel unknown; whole-crop frame bytes must be withheld.
9. Installing the LUT alone does not retroactively make that pixel known.
10. Repainting that pixel restores six known pixels, with the other five
    preserved.

Both ordinary and forced-legacy SPI paths run these stages. The named
`omit_replacement` negative skips only the first replacement upload. It must
complete stage four with the original colours and fail to match the intended
replacement-pixel image. This is an explicit asserted counterexample, not an
ignored test or a fabricated controller stall.

The LUT constants are independently chosen six-bit channels 17,29,43; their
literal eight-bit expansions are 69,117,174. The MX windows use logical
columns 120 and 119 for physical columns 11 and 12 in 132-column GM00 RAM.
Expected memory colours and addresses do not call the model mapping or decoder.

## Acceptance and exclusions

Require all eight tests in `crates/core/tests/st7735_sam_guest.rs` to execute
with zero failures, ignored or filtered tests, both feature-off and
`event-scheduler`. Preserve original capture hashes and all existing displays,
live driver checks, workspace/browser checks and enabled gates. Record exact
source, guest hash, tested checkout and landed tree; formatting/source review
does not qualify execution.

This proves only the documented model contract through authored SAM guest MMIO,
not physical reset timing, partial-table silicon contents, production Image
layout, actual module/aperture/GM/BGR, original native runtime, DMA/IRQ,
active-frame RTx, WASM or installed-app adoption. Invalid-address ignore
semantics and deployed configuration remain separate P3 gates. P3/CP14 remain
open.
