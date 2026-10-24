# ST7735 production evidence lanes — 2026-10-08

Start from refreshed main and inspect open PRs before claiming ownership.
Preserve existing codec/GPIO, rectangular, stock-CODAL stream, all-eight GM00
orientation, retention, invalid-range and PXT caller controls. The
[caller receipt](../receipts/2026-10-08-st7735-pxt-caller/README.md) is component
proof, not completion of [P3](target-next-lanes.md#p3--st7735-stream-panel-and-gpio-observation-foundation-landed-open).
Follow these bounded lanes in order; record unknowns instead of guessing.

## S1 — identify an actual public production build

Inputs: the pinned sources in the [panel guide](../boards/pybadge-native.md)
and an owned or publicly redistributable PyBadge firmware build. Do not fetch
restricted firmware to fill gaps. Record immutable runtime/dependency pins,
compiler and build commands, effective `USE_RGB444`, actual loaded CF2 board ID,
width/height, `DISPLAY_CFG0/1/2`, and selected display constructor/path. A source
default or a harness-defined macro is not deployment evidence.

Files: add a compact build/config receipt under `docs/receipts/`; adjust the
hosted qualification workflow only if needed to reproduce that exact build.
Retain attribution and redistribution terms. No consumer pin update yet.

Acceptance: another agent can reproduce the artifact and its configuration,
verify its hashes and distinguish compiler facts, loaded data and observed path
selection. If no lawful build/config evidence is available, publish that
specific gap and complete preparation without declaring P3 done.

## S2 — bind the actual module and colour contract

Inputs: S1 plus independently sourced public module documentation or operator
captures for GM straps, aperture/offsets, BGR/inversion and RGB444 LUT behavior.
Arcada BLACKTAB/rotation defaults and a PCB symbol alone do not identify the
installed module. Request missing independent inputs; never invent straps or
an identity/default LUT. The direct RGB444 method does not upload RGBSET.

Files: panel guide, an explicitly selected production device/system descriptor
only after evidence supports it, and corresponding authored guest controls.
Keep the unregistered GM00 fixture separate from production PyBadge.

Acceptance: source-bound four-corner/non-square vectors in every advertised
orientation, wrong-window/short-LUT/reset negatives, and explicit separation of
controller-memory RGB666/expanded RGB888 from glass colour. New model changes
require affected clean hosted guest checks and every enabled final-head gate.
Physical or analogue claims require independent capture, not model agreement.

## S3 — establish supported image/status dimensions

Inputs: S1's actual caller/runtime and S2's panel contract. The component harness
observed padding selection for partial/status frames and rejected a synthetic
allocation at an authored boundary; neither establishes a production defect.
Determine actual allowed dimensions, status-bar policy, allocation capacity and
fallback behavior before proposing a fix. Execute original allocation/caller
boundaries where feasible, preserving source notices and invocation details.

Files: caller qualification scripts/tests, a source-bound receipt, and only if a
real supported defect is demonstrated, a separately reviewed change in our own
runtime fork. Do not post upstream or repack fixture pixels to conceal output.

Acceptance: a public supported-case reproducer and independent expected bytes;
any correction fails on the unmodified affected path and passes after the fix.
Retain padding/overflow/reentry negatives and unchanged original captures.
If inputs are unsupported, document the enforced restriction rather than
claiming a production fix. All affected hosted gates must pass before merge.

## S4 — qualify blocking integration, then advance remaining engine lanes

Inputs: S1–S3 and existing original-driver captures. Run a clean guest through
actual MMIO with the justified configuration, non-square full-frame corners,
input press/release, reset/backlight and explicit failure controls. Separate
authored blocking guest proof from original runtime execution. Do not patch or
host-inject completion/pixels to obtain success.

Acceptance: exact source/compiler/guest/descriptor hashes, hosted observations,
all enabled final-head checks and remaining exclusions. Close only the proven
P3 boundary. Then implement separate [P2 IRQ and P4 DMA lanes](target-next-lanes.md),
followed by P5 original runtime, active performance and independently qualified
consumer adoption. Do not rerun idle spin to claim active-frame speed or move
hardware ACKs, baselines, tolerances or app pins on component tests alone.

The [latest archived native spin snapshot](../receipts/2026-10-07-st7735-main-d102/README.md)
belongs to `d102b42`, not current main: PyBadge median 61.036527x. No new active
PyBadge, browser-WASM, installed-app or controlled A/B result follows from S1–S4
preparation. M1 micro:bit work remains a separate lane.
