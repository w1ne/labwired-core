# LabWired catalog sync — `@labwired/catalog`

Date: 2026-06-22
Status: Approved design, pending plan

## Problem

When a new hardware component is added in LabWired, proto.cat does not learn about
it. proto.cat carries a hand-written block catalog at
`protocat/lib/labwired/blocks.ts` (today: one chip, three components). Every time a
peripheral is added or a `device_type` is renamed in LabWired, that file drifts
silently. A drifted `device_type` produces a System Manifest the simulator rejects,
and a missing component is simply invisible to the composer. The fix is to stop
hand-maintaining the parts of `blocks.ts` that LabWired already owns, and to make
proto.cat consume a published artifact that regenerates whenever a component is added.

## Key insight: proto.cat blocks are a superset, not a copy

proto.cat's block definitions mix two kinds of data:

- **LabWired hard facts** — the values that must match the simulator or the device
  breaks: `device_type` (== manifest `external_devices[].type`), chip `id`
  (== embedded chip-catalog id == compile-service `chip_family`), whether the block
  is electrical/simulated, and its transport.
- **proto.cat authoring metadata** — `enclosure_note`, `size_mm`, `lib_deps`,
  `driver_hint`, `wiring_hint`, `verify`, the display/sensor/power `category`, the
  chip-bus binding (`bus`), and `default_cs_pin`. None of this exists in LabWired's
  manifest. It is genuinely new human knowledge (3D-enclosure and firmware-drafting
  domain) that cannot be auto-derived for a brand-new panel.

The current `manifest.json` confirms the boundary: it carries only `device_type`,
`label`, `transport`/`category` (bus type), `config_keys`, and `labs` — no dims, no
lib_deps, no enclosure data. Therefore "auto-publish keeps everything in sync"
cannot mean "copy the whole block". It splits into two honest jobs:

1. **Hard facts** flow automatically from LabWired so a rename or addition can never
   silently break proto.cat's sim path.
2. **Authoring overlay** stays human-authored; automation's only job is to *flag*
   when a new component lacks one.

## Architecture

### Producer: `@labwired/catalog` (new, data-only)

A new package `packages/catalog/` published as `@labwired/catalog`. No React, no
wasm — only data and a few typed helpers, so proto.cat pulls nothing heavy.

It publishes the authoritative hard facts:

- **Peripheral facts**, generated from the existing
  `packages/ui/src/peripherals/manifest.json` (whose own source of truth is the Rust
  `KITS` registry via `gen-peripherals-manifest`): `{ device_type, label, transport }`
  per peripheral.
- **Chip facts**: the embedded chip-catalog ids (`chip_family`).
- Typed exports: `CATALOG_FACTS` (the generated object), `isKnownDeviceType(t)`,
  `isKnownChip(id)`.

Generation mirrors the proven manifest pattern:

- `npm run generate` reads `manifest.json` + the chip-catalog source and writes
  `src/catalog-facts.json` (committed).
- `npm run check` regenerates into memory and diffs against the committed file,
  exiting non-zero on drift — the same gate shape as
  `packages/board-config/test/catalog-regression.test.ts`. This runs in the existing
  core-integrity / build gate so the package can never drift from the registry.
- `npm run build` bundles `src/index.ts` to `dist/` (esbuild, same recipe as
  `@labwired/board-config`).

The package owns **only** the three fields plus the set of valid ids. That is the
ceiling of what the manifest can authoritatively provide; everything else is overlay.

### Consumer: proto.cat overlay + drift gate

`protocat/lib/labwired/blocks.ts` keeps its superset `ChipBlock` / `ComponentBlock`
definitions, but they become an **overlay** validated against `@labwired/catalog`
(added as an npm dependency). A new vitest drift gate in proto.cat enforces both
directions:

1. **No stale references.** Every electrical `ComponentBlock.device_type` and every
   `ChipBlock.id` must exist in `CATALOG_FACTS`. Catches LabWired renames/removals
   that would otherwise produce a simulator-rejected manifest.
2. **No unmapped components.** Every catalog `device_type` must have a proto.cat
   block, minus an explicit `UNSUPPORTED` allowlist. Catches a newly added LabWired
   component proto.cat has not yet given an overlay, failing with the missing id(s).

Mechanical-only blocks (e.g. the LiPo cell) live solely in proto.cat — they are not
in the LabWired manifest — and are excluded from direction (2) by the existing
`simulated: false` filter.

## Auto-version + publish flow

New workflow `packages/catalog` publisher, `.github/workflows/catalog-publish.yml`,
triggered on push to **main** touching `packages/catalog/**`,
`packages/ui/src/peripherals/manifest.json`, or the chip-catalog source.

Steps:

1. Regenerate `catalog-facts.json`. If the generated content is unchanged from what
   is already published, stop (no-op publish guard).
2. `npm version patch` in `packages/catalog`, then
   `npm publish --provenance --access public` using the existing `NPM_TOKEN` secret
   (same setup as `mcp-publish.yml`). The version bump commit is pushed back to main
   with a skip-ci guard to avoid a publish loop.
3. Open or update a **bump PR in the proto.cat repo** raising the
   `@labwired/catalog` dependency to the new version.

The bump PR runs proto.cat's drift gate. It goes green only when proto.cat is
genuinely back in sync. A newly added component with no overlay leaves the PR red —
which is the intended signal that a human must author the enclosure/firmware
metadata before the new component is usable in proto.cat.

## Explicitly NOT synced (scope boundary)

These remain proto.cat-owned and human-authored; automation never invents them, only
flags a missing overlay: `enclosure_note`, `size_mm`, `lib_deps`, `driver_hint`,
`wiring_hint`, `verify`, `category`, `bus`, `default_cs_pin`. Mechanical-only blocks
stay proto.cat-only.

## Components and boundaries

| Unit | Responsibility | Depends on |
| --- | --- | --- |
| `@labwired/catalog` generator | Turn `manifest.json` + chip catalog into `catalog-facts.json` | manifest.json, chip-catalog source |
| `@labwired/catalog` package API | Typed facts + `isKnownDeviceType`/`isKnownChip` | generated json |
| `catalog-publish.yml` | Detect change, version, publish, open proto.cat bump PR | package, NPM_TOKEN, proto.cat repo |
| proto.cat `blocks.ts` overlay | Superset block defs (hard facts + authoring metadata) | `@labwired/catalog` |
| proto.cat drift gate (vitest) | Two-direction consistency check | `@labwired/catalog`, blocks.ts |

## Testing

- `@labwired/catalog`: `check` gate (generated == committed) wired into the build
  gate; a unit test for `isKnownDeviceType`/`isKnownChip`.
- proto.cat: the two-direction drift gate as a vitest test in proto.cat CI.
- The publish workflow's no-op guard verified by a dry-run path (regenerate, compare,
  skip when unchanged).

## Risks / open questions

- **Version-bump commit loop.** Pushing the bump commit back to main must be guarded
  (path filter + skip-ci) so it does not retrigger the publisher. To finalize in the
  plan.
- **proto.cat repo target.** The bump PR targets `protocat` (the public consumer
  repo); confirm whether `protocat-private` also needs the bump or inherits it.
- **Cross-repo PR auth.** Opening a PR in proto.cat from the LabWired workflow needs a
  token with access to that repo; to specify in the plan.
