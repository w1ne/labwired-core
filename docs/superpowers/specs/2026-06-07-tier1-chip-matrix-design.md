# Tier-1 Chip-Regression Validation Matrix — Design

**Date:** 2026-06-07
**Repo affected:** `core` (submodule); scoreboard surfaces in core docs site
**Status:** approved design, pending implementation plan

## Problem

"Supported chip" is currently proven unevenly. The full workspace test pass runs
real firmware on ~6 chips well (L476, nRF52x, ESP32-family via `firmware_survival`
and the e2e suite), but stm32g474re, stm32wb55, stm32wba52, stm32l073 and others
have chip YAMLs and catalog entries with no real-firmware evidence in the per-PR
gate. The L0–L3 target-support rubric (`docs/target_support_rubric.md`) defines a
Tier-1 peripheral checklist — `clock/rcc`, `gpio`, `uart`, `timer`, `dma`, `irq` —
but nothing in CI enforces it, and the SVD register-coverage ratchet protects only
ESP32-S3. Net effect: a core upgrade can silently break a chip or a peripheral on
a chip and CI stays green.

## Prior art consulted

- **Renode Zephyr dashboard** — runs Zephyr samples on 470+ simulated boards
  nightly; per-board color-coded statuses; platform definitions derived from
  devicetree. Takeaway: a small set of well-chosen firmware images validates a
  peripheral class each; breadth comes from an ecosystem corpus, not hand-written
  tests.
- **Renode per-platform Robot suites** (e.g. `tests/platforms/NRF52840.robot`) —
  13 cases per platform exercising UART shell, SPI/I2C sensors, GPIO LED/button,
  watchdog; binaries precompiled, content-hashed, fetched from
  `dl.antmicro.com`. Takeaway: CI never installs cross-toolchains; ELFs are
  pinned by hash; assertions are UART strings + GPIO state.
- **In-repo precedents** — `svd_coverage_ratchet.rs` (floor that can only rise),
  `firmware_survival.rs` (real-FW harness with UART/PC assertions),
  `catalog_validation.md` + `core-validate-hw-targets.yml` (CI writes
  `pass_rate`/`validation.*` into onboarding manifests),
  `generate_coverage_matrix_scoreboard.py` (rendered scoreboard).

## Design

### 1. Matrix model

The unit of truth is a **cell**: `(chip, peripheral-class) → status`.

- Peripheral classes = exactly the rubric's Tier-1 list:
  `clock/rcc`, `gpio`, `uart`, `timer`, `dma`, `irq`.
- Status ∈ `pass | partial | blocked | n/a | unrecorded`.
- Rows = the chip YAMLs in `configs/chips/`. A chip whose YAML declares no
  peripheral of a class gets `n/a` for that cell (not `blocked`).
- Committed snapshot: `docs/coverage/tier1-matrix.json` (same pattern and
  directory as `esp32s3-coverage.json`).
- A chip's rubric L-level is **derived** from its row (all six `pass` → eligible
  for L3), not hand-asserted.

### 2. Tier-1 fixture firmware

One small bare-metal firmware per **chip family**, not per chip (~5 sources):

| family source | chips covered |
|---|---|
| `stm32` (variants via cfg) | f103, f401, f401cdu6, f407, g474re, h563, l073, l476, wb55, wba52 |
| `nrf52` | nrf52832, nrf52840 |
| `rp2040` | rp2040 |
| `esp32-riscv` | esp32c3 |
| `esp32-xtensa` | esp32, esp32s3, esp32s3-zero |

Each fixture runs a self-test sequence and reports one line per peripheral class
over UART:

```
TIER1 clock PASS
TIER1 gpio PASS
TIER1 timer PASS
TIER1 dma FAIL code=<reason>
TIER1 irq PASS
TIER1 done
```

Conventions:

- UART is validated implicitly: no `TIER1` lines within the step budget →
  `uart = blocked` for that chip.
- `TIER1 done` is mandatory; missing `done` marks the whole row `partial`
  (firmware hung mid-sequence).
- Deterministic: fixed step budget per chip, no wall-clock dependence; budgets
  recorded next to the chip entry in the harness table.

**Binary policy** (per explicit user decision): ELFs are **committed in-repo**,
content-hashed:

- `crates/core/tests/fixtures/tier1/<chip>.elf`
- `crates/core/tests/fixtures/tier1/MANIFEST.json` — sha256 + source revision
  per ELF
- Sources in `examples/tier1-fixture/<family>/`
- A scheduled toolchain-equipped CI job rebuilds from source and fails on
  source↔binary drift (weekly; Xtensa rides the espressif toolchain runner).

### 3. Harness

New integration test `crates/core/tests/tier1_matrix.rs`, structured like
`firmware_survival`:

1. Table of `(chip, system-yaml, elf, step-budget)`.
2. Run each ELF against its system YAML; capture UART.
3. Parse `TIER1` lines → cell statuses; merge `n/a` from chip YAML peripheral
   declarations.
4. Expose the live matrix for the ratchet test and the CLI exporter.

CLI exporter (mirrors the SVD coverage exporter):

```
cargo run -p labwired-cli -- tier1-matrix --json-out docs/coverage/tier1-matrix.json
```

### 4. Ratchet gating

New test `tier1_matrix_ratchet` (same shape as `svd_coverage_ratchet`):

- Any recorded `pass` cell that no longer passes → **test fails, PR blocked**;
  failure message names exact cells and the regenerate command.
- `unrecorded`, `partial`, `blocked` cells move freely — adding chips never
  blocks anyone.
- Improving a cell = regenerate the snapshot in the same PR; progress is
  recorded and immediately protected.
- Demotion happens only by editing `tier1-matrix.json` in a PR — visible in
  diff review; satisfies the rubric's demotion rules with no side channel.

**SVD coverage ratchet extension:** same mechanism as today's ESP32-S3 ratchet,
extended chip-by-chip as SVD snapshots are generated (incremental opt-in; not a
P1 blocker for chips without an in-tree SVD).

### 5. CI wiring

- **Per-PR:** `tier1_matrix` + ratchet run inside the existing `core-integrity`
  workspace test pass. No new workflow, no toolchains (ELFs committed). Cost:
  ~17 bounded sim smokes, seconds each.
- **Nightly** (`core-nightly.yml`): full matrix with generous budgets + drift
  check job.
- **Catalog refresh** (`core-validate-hw-targets.yml`): writes per-chip Tier-1
  results into onboarding manifests' `validation.checks`, replacing the generic
  `simulation: true` with per-peripheral verdicts. Downstream catalog consumers
  keep the same contract (`catalog_validation.md`), with richer checks.

### 6. Dashboard

Two surfaces, one source of truth (`docs/coverage/tier1-matrix.json` on core
main, refreshed nightly with `run_url` evidence):

1. **Repo scoreboard** — a chip × peripheral markdown grid
   (✅ pass / 🟡 partial / ⛔ blocked / — n/a) rendered into
   `docs/coverage/tier1-scoreboard.md`, linked from `docs/coverage_scoreboard.md`.
2. **Website scoreboard (the public trace)** — a styled validation-matrix page
   on the playground site (the `/ci` audience), fetching the raw
   `tier1-matrix.json` from core main client-side so it is always as fresh as
   the last nightly run. This page is the outreach link for the
   driver-bringup-CI beachhead claim.

On both surfaces every cell links its CI `run_url` (stored alongside the
status in `tier1-matrix.json`); cells without evidence render as `unrecorded`
regardless of local results.

## Wedge alignment (wedge v2, 2026-06-06 moat doc §5)

The matrix is built **beachhead-first**, not coverage-first:

1. **P1 targets the named beachhead.** The wedge's beachhead is driver-bringup
   CI for the ESP32-S3 peripherals QEMU stubs and Wokwi lacks (MCPWM, I2C/SENS,
   RMT). The S3 row of this matrix — riding the vendored-ROM faithful boot — IS
   the public proof surface for that claim. STM32 breadth follows; it serves
   trust, not the wedge.
2. **Proof-artifact bar applies to the scoreboard.** Every non-`unrecorded`
   cell links to its evidence: the CI `run_url` (and artifact bundle) that
   produced it, same contract as the onboarding manifests. A green cell with no
   linked run does not render green. No claim without its artifact.
3. **Moat boundary stays intact.** Public = the Tier-1 bring-up fixtures, their
   sources, and the matrix (trust surface). Private = the silicon-captured
   golden-trace corpus, hw-oracle captures, and any customer-firmware
   regression corpora — the excludable, paid layer (hosted
   golden-trace/regression-corpus service). This matrix must never grow
   silicon-trace-derived golden outputs as public fixtures.
4. **Beachhead extension for S3 only:** beyond the six rubric classes, the S3
   row adds `mcpwm`, `i2c`, `rmt` cells (the beachhead peripherals), validated
   by the same fixture. Other chips stay at the six rubric classes until the
   wedge expands.

## Phasing

- **P1 (wedge)** — harness + ratchet + exporter + scoreboard + **ESP32-S3
  Xtensa fixture** (esp32s3, esp32s3-zero; classic esp32 variant included),
  six rubric classes + `mcpwm`/`i2c`/`rmt` beachhead cells, riding the
  vendored-ROM faithful boot path. Output doubles as the driver-bringup-CI
  proof page for outreach.
- **P2** — ESP32-C3 + STM32-family fixture (10 chips, incl. all
  currently-untested STM32s).
- **P3** — nRF52, RP2040 fixtures.
- **P4** — Zephyr-samples breadth layer per board (Renode-style); separate spec.

## Error handling

- Fixture ELF missing/corrupt (hash mismatch) → harness fails that chip's row as
  `blocked` with a distinct reason; ratchet then reports it only if the row had
  recorded passes.
- Sim crash/hang → step budget exhausts; row marked from whatever `TIER1` lines
  arrived; missing `done` → `partial`.
- Ratchet snapshot regeneration is idempotent and deterministic; running it
  twice produces identical JSON (sorted keys).

## Testing

- Harness unit tests: UART-line parser (PASS/FAIL/garbage/truncated), `n/a`
  derivation from chip YAML, budget exhaustion paths.
- Ratchet tests: regression detected, improvement allowed, unrecorded ignored,
  explicit-demotion diff respected.
- Drift check: corrupting one committed ELF locally must fail the manifest
  verification.
- The matrix harness itself is the regression test for the 17 chips.

## Out of scope

- Zephyr corpus integration (P4, separate spec).
- Non-Tier-1 peripherals (SPI/I2C/ADC device labs remain covered by
  `firmware_survival` / e2e suites).
- HIL/silicon capture — the hw-oracle pipeline is untouched.

## Amendment — overview vs. detail report (2026-06-07, post-P3)

Owner direction after the full 15-chip matrix shipped:

1. **Overview table = the 12 universal subsystems only** (clock, gpio, uart,
   timer, dma, irq, i2c, spi, adc, pwm, wdt, rtc). Chip-specific classes (rmt,
   twai, i2s, usb, …) leave the top-level grid.
2. **`na` relabels honestly.** For universal subsystems every silicon has the
   feature, so an undeclared class is a *model gap*, not "not applicable" —
   render 🚧 "not modeled" (distinct from ⛔ model-broken and · check-not-written).
   True n/a remains only for chip-specific classes on chips whose silicon lacks
   them.
3. **Per-chip detail report (click-through on the page; sections in the md
   scoreboard)** with *instance-level* honesty — each MCU has UART1/2/3 etc.;
   the report states exactly what is modeled:
   - per class, three lists: **modeled** instances (chip yaml ids / programmatic
     wiring), **validated** instance(s) (per-target metadata maintained beside
     the fixture), **on-silicon** inventory (machine-derived from the vendor SVD
     via svd-ingestor where an SVD is in-tree; "inventory pending" otherwise —
     never a hand-asserted complete list).
   - per cell: the check performed, FAIL `code=` (parser to retain it in the
     snapshot), evidence link.
4. Page drill-down ships as expandable rows first; linkable per-chip pages
   (/validation/<chip>) when outreach needs them.

Sequencing: lands as its own slice after the model-bugfix integration PR
(bit-band / thumb / gdma / c3-timer) and its pending-silicon-verification doc.
