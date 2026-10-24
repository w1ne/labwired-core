# Original PXT caller/Image fragments — qualified component execution

This extension executes exact SHA-bound fragments from the same
Microsoft MIT PXT source pin as the [qualified direct method](README.md).
Both caller builds and the original method passed in [run 37728675697](https://github.com/CrispStrobe/labwired-core/actions/runs/37728675697).
See the [source-bound receipt](../../docs/receipts/2026-10-08-st7735-pxt-caller/README.md)
for admission controls, independent byte audit and exact merge identity.

The inputs add [pxtbase.h](https://github.com/microsoft/pxt-common-packages/blob/31abf23d118f35010fb75122e60a1eb2b6dffe9f/libs/base/pxtbase.h).
The original ImageHeader, Image accessors, updateScreen function and RGB444
method are extracted byte-for-byte and checked against individual required
hashes before compilation. The original source files themselves remain
SHA-bound. Microsoft attribution and full MIT notice remain in README.md and
the exact downloaded license accompanies the hosted artifact.

Object allocation, BoxedBuffer, WDisplay fields/palette, SPI/pins, configuration,
waits, window calls and fallback driver are authored boundaries. ImageHeader
and selected original accessors execute inside an authored Image class, not the
original RefImage inheritance, allocator, constructors or GC. The original
updateScreen function executes with those objects. No function bytes are
rewritten, no Image data is transposed/repacked and padding remains nonzero.

## Qualified checks

Compile and execute with USE_RGB444 both enabled and absent. Eleven cases in
each build cover full 160×128, padded 160×5 partial input, aligned 120+8 and
padded 125+3 main/status frames, predicate-off fallback, missing-LCD fallback,
doubled fallback, wrong dimensions, wrong bpp, padded-copy capacity rejection,
and missing-display/reentry rejection.

Distinct column/row pixel values and padding index15 are checked against a
separately calculated storage oracle. All directly emitted bytes are compared
with an independent literal RGB444 palette oracle. The expected padded-copy
sizes are checked separately from the original Image methods. A named host
capture mutant flips the first caller RAMWR byte and must fail with
`caller RAMWR mismatch`.
The artifact retains every RAMWR byte and transfer size for independent audit;
logs print only a short summary. Width160's original two-row batching remains
unchanged and is checked for each main/status frame.

The exact caller's memcpy is intercepted at the authored boundary to record
length and reject an oversized copy before mutation. Destination capacity uses
the source-reviewed constructor formula, but the constructor does **not**
execute. The synthetic 160×1 total-display case therefore tests a boundary
rejection; it is not a deployed PyBadge overflow claim. Preserve the original
panic path's reentry flag rather than silently repairing it in the harness.

Partial-height padding selection is an observed component output in this run.
It does not establish physical misrendering, a deployed
configuration, or permissible production dimensions. Stock fallback calls are
recorded, not original CODAL execution; emitted window methods are authored,
not panel/controller address qualification.

## Acceptance and exclusions

The original method capture, both caller builds, sixteen admission controls,
the corrupt-data negative and all fifteen enabled exact-head checks passed.
PR172 merged with the tested tree unchanged. Preserve original artifact hashes
and notices; do not relabel authored boundaries as original runtime execution.

This remains component-level host x86 execution, not full PXT/RefImage/Image
allocation, original constructor/CF2 selection, ARM/SAM/DMA/IRQ, deployed module
GM/aperture/LUT, panel colour equivalence, RTx, browser or installed-app proof.
The [P3 panel contract](../../docs/boards/pybadge-native.md) remains open.
Continue with the [production evidence lanes](../../docs/engineering/st7735-production-next-lanes.md).
