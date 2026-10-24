# ST7735 colour pipeline — board integration still pending

`crates/core/src/peripherals/components/st7735_color.rs` is a pure byte codec.
The draft generic-display integration opts into it using `serial_color` and
`load_rgb_lut`; it is not a shipped PyBadge display or
performance result. Existing fixed-format panels do not opt in, and the
ST7789 descriptor and PyBadge configuration remain unchanged.
The full acceptance boundary remains [P3](target-next-lanes.md) and the
[PyBadge panel contract](../boards/pybadge-native.md).

The implementation uses the [ST7735R v0.2 datasheet](https://cdn-shop.adafruit.com/datasheets/ST7735R_V0.2.pdf),
particularly sections 9.8.20–22, 9.15, 9.18 and 10.1.22. It decodes serial
RGB444/RGB565/RGB666 into six-bit memory channels. RGBSET has 128 entries;
undefined lookup values produce an explicit unknown pixel, not invented black.
Complete pixels remain distinguishable from incomplete bytes. Software reset
preserves depth/table; hardware reset restores depth and invalidates the table.
Reserved COLMOD rejection is an emulator API policy, not silicon qualification.

Unit controls cover all 4,096 RGB444 and 65,536 RGB565 inputs with a non-linear,
channel-distinct table, RGB666 low-bit exclusion, delivery chunk splits, partial
pixel discard, table invalidation/replacement and reset/depth changes. These
controls are not evidence of a working native screen. Hosted checks must pass
on the exact source before adopting the codec.

`.github/workflows/st7735-color-controls.yml` compiles this dependency-free
module directly with Rust 1.95.0 and requires all eleven controls to be listed
before execution. This focused hosted check supplements, not replaces, the
normal crate/workspace checks; it does not execute a guest or measure RTx.

## Draft parser and memory integration

`crates/core/tests/fixtures/st7735r-memory.yaml` is an **unregistered test
prototype** using GM00's
132×162 controller extent, not a claim about PyBadge module GM/crop. It must be
loaded explicitly. It is deliberately outside the production device catalogue:
every `configs/devices` descriptor must be a registered kit, and this fixture
must not become a shipped panel before qualification. The production YAML
baseline stays at main's 89; no gate exemption or model allowance is added.
The original location failed `every_device_descriptor_is_a_kit` at source
`8a810a86b1958a071de109128b139fd274a61dec` in
[board run 37603425496](https://github.com/CrispStrobe/labwired-core/actions/runs/37603425496).
Preserve that failure; the fixture relocation still needs exact-head hosted
verification and is not a retrospective passing result.
RGB888 artifact bytes contain bit-expanded RGB666 memory
channels (`(v << 2) | (v >> 4)`), not original wire bytes or analogue glass output.
BGR, gamma, scan order and inversion rendering remain unimplemented. MADCTL
address mapping reuses the generic engine; physical module parity is still owed.

The opt-in parser collects the complete 128-byte RGBSET payload, invalidates
the previous table at upload start, and leaves an interrupted upload undefined.
Whole-byte CS pauses preserve both parameter and pixel progress. A new command
discards an incomplete pixel; each completed pixel consumes one address step,
including a pixel whose lookup value is unknown. Later LUT changes do not
recolour stored RAM. Partial parameter-register updates on silicon are not
claimed by this all-or-nothing command parser.

RAM starts unknown. A validity map follows the same orientation/crop as frame
bytes; artifacts publish `known_pixels` and `unknown_pixels` and withhold bytes
if any visible pixel is unknown, even when bytes were requested. `painted_bytes`
is then null. Raw `framebuffer()` users must consult `known_pixels()` rather than
interpret storage placeholders as valid black. Generation includes validity,
so unknown-to-known black is observable. This policy avoids a fabricated frame,
The shared inspection decoder now accepts RGB888 for region ink measurements
and explicitly rejects unknown-pixel metadata, even if a caller supplies black
placeholders. Short payloads, overflowing RGB geometry/regions and malformed
unknown counts are errors. This is not a physical-colour or browser-rendering
claim; downstream RGB888 rendering still needs integration.

Software reset retains COLMOD, MADCTL, LUT, RAM and pixel validity; hardware
reset restores COLMOD/MADCTL defaults and invalidates the LUT, retaining RAM
and validity. Reset cancels parser fragments and restores orientation-aware
full windows. Optional reset/backlight GPIO attachment is drafted below; it
is not yet qualified on the current source.
Reserved COLMOD values are rejected without changing the selected format and
counted in artifact metadata. Clear-RAM/refresh and incompatible descriptor
formats are rejected for this profile.

Colour snapshots use a separate tagged shape containing RAM and validity.
Existing fixed-format v1 snapshots are unchanged. Cross-profile/wrong-size
restores are refused before mutation. As with existing display snapshots,
control/parser state is rebuilt by bus replay, not restored by this RAM snapshot.

Sixteen `st7735_stream_*` controls in
`crates/core/src/peripherals/components/declarative_display.rs` exercise this
prototype through the SPI door, plus descriptor/validity/snapshot checks.
They run in the normal hosted crate/workspace checks, not the standalone codec
job. Until exact-head checks pass these are proposed controls, not a passing
qualification receipt. No guest or performance result is implied.

## Authored SAM SPI guest — hosted qualification pending

`crates/core/tests/st7735_sam_guest.rs` loads authored Thumb instructions, vector
bytes and literals into the actual Cortex-M machine with the PyBadge chip/system
descriptors. The host attaches the prototype and sets a **fixture-only** visible
2×2 crop; it does not configure guest MMIO, write pixel RAM, replace the guest's
SPI implementation or seed completion. The guest enables MCLK/GCLK, configures
SERCOM4 and PB13/PB15 mux, controls PB7 CS/PB5 D/C, uploads a non-linear RGBSET,
selects RGB444 and a 2×2 window, sends four packed pixels and polls TXC before
releasing CS after each byte. An SRAM marker distinguishes guest completion.

The original two integration tests cover exact memory bytes/region measurement
and separate
MCLK/GCLK/mux/CS/D/C negatives in both ordinary and forced-legacy controller
paths. Clock/mux negatives must stall completion; CS/D/C negatives must complete
controller transmission without painting. The normal CI feature-off and
scheduler jobs invoke this target through the nonvacuous wrapper. Both tests
passed in the feature-on step of [run 37602485369](https://github.com/CrispStrobe/labwired-core/actions/runs/37602485369/job/112730721587)
at source `214473596c13ffd74cf1683367569c99b46dc258`; the superseded overall
run was cancelled. This is a completed two-test step, not a successful aggregate
or qualification of the later GPIO source. Exact-head qualification remains
pending. The original tests prove no real module crop, full native frame,
reset/backlight, IRQ, DMA or RTx result.

## Draft write-driven reset/backlight attachment

The optional `gpio_control` descriptor binds reset and backlight config keys
with explicit polarity. When configured, the generic kit shares one actual
display RAM between the SPI handle and a GPIO observer. It resolves the port
address and bit together, samples both inputs after stores to their GPIO ports,
and adds no per-cycle polling or controller-specific downcast. Missing optional
config keys remain unwired; a production board contract must require its actual
wiring. Existing descriptors without this option keep their previous behavior.

Known pad level requires an actively driven, uncontended pad; a latch value
alone is insufficient. Unknown reset blocks transfers and discards fragments
across state changes. Asserted reset restores control defaults and invalidates
the LUT, retaining RAM/validity. Held reset rejects writes. Unknown or inactive
backlight makes `lit` false but does not suppress RAM writes. Artifacts expose
`reset_asserted` and `backlight_on`, with null for an unknown connected wire.
These are conservative emulator policies, not analogue/timing qualification.

Four added narrow-port unit controls and two added SAM integration controls are
**not yet executed**. The SAM fixture uses the actual generic kit/AttachCtx path
with an explicitly enabled fixture-only crop. Authored instructions drive PA0
reset and PA1 backlight, including held/undriven/muxed reset, a wrong-port bit,
dark-but-painted RAM, and reset after disabling SPI's MCLK. Separate host-MMIO
diagnostics check backlight changes without a tick and contended-pad rejection.
External input/net changes alone do not notify this store-driven observer;
their propagation remains a separate task. No full-frame, physical module,
timing, native Arcade, DMA, IRQ, browser, app adoption or performance claim is
made by this draft.

## Next implementation slice

1. Review and qualify the opt-in parser/memory integration on hosted CI,
   including the existing fixed-format display controls. Keep partial-table
   silicon contents outside the declared boundary unless independently modelled.
2. Qualify RGB888 artifact verification and independently check
   BGR semantics without confusing controller memory with glass colour. Match
   independent vectors to the pinned driver's packed stream.
3. Determine module
   GM/crop before claiming physical four-corner orientation parity.
4. Add generic reset/backlight GPIO observation and SAM SPI attachment without
   controller-specific bus downcasts or new idle polling. Prove mux/clock/CS/DC
   negatives and reset/backlight effects using an owned permissive MMIO guest
   with pixel artifacts on hosted CI. Retain the existing scheduler controls.
5. Only then complete P3 qualification, followed by the separate IRQ/DMA and
   production-runtime lanes. No app pin, hardware acknowledgement or benchmark
   baseline should move on codec unit tests alone.

The pinned [CODAL stream](https://github.com/lancaster-university/codal-core/blob/312ae57e0b31f5b9df07a81e9d846945828e3c5a/source/drivers/ST7735.cpp)
is the later integration input. Independent datasheet vectors and actual driver
bytes must agree before a production-stream claim; no such guest run exists for
this foundation yet.
