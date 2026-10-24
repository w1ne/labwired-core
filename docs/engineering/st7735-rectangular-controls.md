# Rectangular ST7735 blocking controls — qualification pending

This next P3 slice extends `crates/core/tests/st7735_sam_guest.rs`, not the
production model or PyBadge wiring. The existing four tests remain; a fifth
authored Cortex-M test adds a nonzero, non-square controller-memory window and
an intentionally wrong-axis negative, in ordinary and forced-legacy SPI paths.
No local emulator execution or hosted passing result is claimed yet.

## Literal vector and independent expectations

The reviewed public input is
[CODAL ST7735.cpp at 312ae57e](https://github.com/lancaster-university/codal-core/blob/312ae57e0b31f5b9df07a81e9d846945828e3c5a/source/drivers/ST7735.cpp),
SHA256 `d8aafbdd338c9501c314f33983eb1ceb1229bc0acfcd10c28a4d2527d756c805`.
Its `setAddrWindow(7, 11, 3, 2)` describes CASET 11..12 and RASET 7..9.
The driver coordinates are not silently relabelled as physical columns/rows.
With MADCTL at its zero reset value, the fixture's fixed controller-memory crop
is columns 11..12 and rows 7..9 (2×3), not a deployed module aperture.

Ordinary, non-doubled indexed bytes `21 43 65` describe indices 1..6, lower
nibble first. The literal RGB444 wire vector is
`11 12 22 33 34 44 55 56 66`. The guest uploads an independently authored,
channel-distinct nonlinear LUT, not CODAL's production palette. Expected LUT
addresses are literal R/G/B triples `[1,33,97]` through `[6,38,102]`, in physical
row-major order. Neither expected addresses nor wire bytes call the emulator's
mapping/codec helpers. Every byte travels through guest SPI DATA and TXC polling,
with CS paused between bytes, including within parameters and packed pixels.

The negative changes only window dimensions to three columns by two rows.
SPI must still complete, but the declared crop must report four known and two
unknown pixels and withhold frame bytes. This catches an axis/window error that
the older symmetric 2×2 positive cannot expose. No host MMIO setup, pixel RAM
injection, completion seeding or guest-controller replacement is used.

## Acceptance and still-open work

Require all five guest tests to execute and pass in the hosted feature-on and
feature-off jobs, plus all enabled exact-head checks. Record source and guest
hashes, original failed results and tested/landed tree identities before merge.
No floor, catalogue allowance, production descriptor, app pin or capture moves.

This source review and literal authored trace do **not** execute the compiled
CODAL driver. Continue the [full P3 follow-on contract](st7735-color-foundation.md#follow-on-lane-rectangular-production-stream-and-panel-binding):
actual driver extraction/execution, all advertised orientations, odd-pixel
command interruption, retained RAM under orientation/LUT/reset changes, deployed
CF2/macros and module aperture/GM evidence. BGR/glass rendering, IRQ/DMA, native
Arcade, active-frame RTx, new-panel WASM rendering and consumer adoption remain
unqualified. A single passing rectangular guest must not mark P3 or CP14 done.

## Referenced source notice

The vector is authored from the referenced public protocol layout; preserve the
source's notice when carrying driver-derived material forward:

MIT License

Copyright (c) 2017 Lancaster University

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
