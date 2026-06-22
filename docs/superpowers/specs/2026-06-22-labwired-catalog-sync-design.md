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
(added as an npm dependency). The drift gate is a standalone `tsx` script
(`check-catalog-sync.ts`) wired into proto.cat CI — proto.cat has no unit-test
runner, only Playwright. It enforces two directions with different severities:

1. **No stale references (hard fail).** Every electrical `ComponentBlock.device_type`
   and every `ChipBlock.id` must exist in the published facts. Catches LabWired
   renames/removals that would otherwise produce a simulator-rejected manifest.
2. **No *newly* unmapped peripheral (hard fail), backlog is informational.** proto.cat
   commits an auto-generated `coverage-baseline.json` listing the peripherals it
   knowingly does not map yet. The gate fails only for a peripheral that is neither
   mapped nor in the baseline — i.e. one that appeared in LabWired *after* adoption.
   The acknowledged backlog prints as a note, not an error.

This split is deliberate (it answers the "auto-sync is really auto-nag" critique):
the hard facts genuinely auto-sync, and a brand-new component is force-flagged, but
proto.cat is not nagged into covering all 22 peripherals at once via a hand-typed
allowlist. Coverage stays human-paced yet drift-proof.

Mechanical-only blocks (e.g. the LiPo cell) live solely in proto.cat — they are not
external peripherals — and never enter direction (2) because the coverage set is
`PERIPHERAL_DEVICE_TYPES` (external devices only).

## Auto-version + publish flow

New workflow `packages/catalog` publisher, `.github/workflows/catalog-publish.yml`,
triggered on push to **main** touching `packages/catalog/**`,
`packages/ui/src/peripherals/manifest.json`, or the chip-catalog source.

Versioning is **committed, not ephemeral** (answering the "version lies" critique).
The package publishes the version committed in `packages/catalog/package.json`; a PR
gate in Playground CI fails if `catalog-facts.json` changes without a version bump,
so whoever adds a component bumps the version in the same PR. This keeps the
committed version honest with no privileged push-to-main token.

Steps:

1. On push to **main** changing `catalog-facts.json`, publish the committed version
   with `npm publish --provenance --access public` (existing `NPM_TOKEN`), skipping
   if that exact version is already on npm.
2. Open or update a **bump PR in `w1ne/protocat`** raising the `@labwired/catalog`
   dependency to the published version (needs a `PROTOCAT_PAT` secret).

The bump PR runs proto.cat's drift gate. Direction-1 drift or a brand-new unmapped
peripheral leaves it red — the signal a human must map or acknowledge the component.
The acknowledged backlog does not block the PR.

## Explicitly NOT synced (scope boundary)

These remain proto.cat-owned and human-authored; automation never invents them, only
flags a missing overlay: `enclosure_note`, `size_mm`, `lib_deps`, `driver_hint`,
`wiring_hint`, `verify`, `category`, `bus`, `default_cs_pin`. Mechanical-only blocks
stay proto.cat-only.

## Components and boundaries

| Unit | Responsibility | Depends on |
| --- | --- | --- |
| `@labwired/catalog` generator | Turn `CATALOG` + `manifest.json` + boards/pin-maps into `catalog-facts.json` | board-config + ui manifest source |
| `@labwired/catalog` package API | Typed facts + `isKnownDeviceType`/`isKnownChip` + `schemaMatches`/`assertSchemaCompatible` | generated json |
| `catalog-publish.yml` | Publish committed version on facts change, open proto.cat bump PR | package, NPM_TOKEN, PROTOCAT_PAT |
| proto.cat `blocks.ts` overlay | Superset block defs (hard facts + authoring metadata) | `@labwired/catalog` |
| proto.cat drift gate (`tsx` script) | Direction-1 + new-peripheral check vs coverage baseline | `@labwired/catalog`, blocks.ts |

## Testing

- `@labwired/catalog`: `check:facts` drift gate (generated == committed) + unit tests
  for the helpers, coverage-set membership, chip-alias exclusion, and schema
  tolerance — wired into Playground CI.
- proto.cat: the drift gate (`check:catalog`) run as a CI step.
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
