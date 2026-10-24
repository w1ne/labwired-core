# Pinned CODAL host trace — qualification pending

This harness compiles the unmodified `ST7735.cpp` and `ST7735.h` from CODAL pin
`312ae57e0b31f5b9df07a81e9d846945828e3c5a`. Both downloads require fixed SHA256
matches before compilation. The original header retains Lancaster University's
2017 MIT notice. Inputs are public source only, not private firmware images.

Hosted workflow `.github/workflows/st7735-codal-trace.yml` compiles a 32-bit x86
executable, matching the source's pointer/unsigned width assumption without
`-fpermissive`. The original driver calls an explicitly artificial ScreenIO/Pin/
event boundary. Callback delivery is bounded and cooperative; a pending fiber
wait fails rather than pretending transfer completion. This is **not** a SAM
controller, DMA, interrupt, physical reset delay or ARM runtime implementation.

Three literal source vectors cover widths 3/4/5 at height 2: byte packing, word
packing and word-plus-tail packing. Independent expectations check nonzero
CASET/RASET bounds, all 128 RGBSET bytes (including unused zero entries), and
every RAMWR byte. The selected palette includes distinct channels and a
non-extreme colour. CS must be active for every captured byte and released at
completion. The artifact records exact source/header hashes and all commands.
A separate named host-capture mutant corrupts CASET and must fail specifically
with a window mismatch. It validates the oracle, not a SAM hardware negative.

Acceptance: the hosted job must execute all three cases and pass on the exact
reviewed source, together with every enabled repository check. No passing run
is claimed by this draft. Preserve compiler/trace failures rather than relaxing
the vector or replacing the source algorithms. The existing authored SAM guest
remains independently qualified; this host trace does not replace it.

Next: bind the actual captured driver bytes/palette to an authored blocking-MMIO
guest; test orientations and odd-pixel interruption/retention; establish deployed
CF2/macros and module aperture/GM. Keep actual driver byte evidence separate
from native DMA/fiber execution, glass/BGR, active-frame RTx and consumer pins.
See `docs/engineering/st7735-color-foundation.md` and
`docs/engineering/st7735-rectangular-controls.md`.
