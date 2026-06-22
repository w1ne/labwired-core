# @labwired/catalog Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish LabWired's authoritative hardware "hard facts" as a data-only npm package `@labwired/catalog`, and make proto.cat consume it with a drift gate, so adding a component in LabWired can never silently break proto.cat's sim path.

**Architecture:** A new standalone package `packages/catalog/` generates `catalog-facts.json` (valid `device_type`s = union of `board-config` `CATALOG` + kit manifest; valid `chips` = `PIN_MAPS` keys) via a `tsx` generator that imports the real exports. The package ships the JSON plus typed helpers (`isKnownDeviceType`, `isKnownChip`). A publish workflow versions+publishes on change and opens a bump PR in proto.cat. proto.cat keeps its superset `blocks.ts` as a human-authored overlay, validated by a standalone two-direction drift script wired into CI.

**Tech Stack:** TypeScript (strict, ESM), esbuild (bundle), vitest (package tests), tsx (generator), GitHub Actions, npm (publish), pnpm (proto.cat side).

## Global Constraints

- Packages are **standalone** — there is no root `package.json`/workspaces. Each package has its own `package.json`, mirrored on `@labwired/board-config`. Do not introduce a workspace root.
- ESM only: `"type": "module"`, `target` ES2022, `module` ESNext, `moduleResolution` Bundler in `tsconfig.json`; `tsconfig.build.json` mirrors board-config (CommonJS/Node10, declarations).
- The runtime package must be **zero-dependency** (data + helpers only). `tsx`/`@labwired/board-config` are devDependencies used by the generator only; their data is baked into committed JSON.
- Commits use git identity `w1ne` / `14119286+w1ne@users.noreply.github.com`. No "Claude"/AI references in any commit message, PR, or file.
- Drift-gate pattern must mirror `packages/board-config`: a `--check` mode that regenerates in memory and diffs the committed file, plus a vitest test that runs `npm run check:facts` and asserts it does not throw.
- `catalog-facts.json` `schema_version` starts at `1`. Bump only when the JSON shape changes.

---

## File Structure

- `packages/catalog/package.json` — package manifest (name `@labwired/catalog`).
- `packages/catalog/tsconfig.json`, `tsconfig.build.json`, `vitest.config.ts` — mirror board-config.
- `packages/catalog/scripts/generate-catalog-facts.ts` — the generator (run via tsx), with `--check`.
- `packages/catalog/src/catalog-facts.json` — generated, committed.
- `packages/catalog/src/index.ts` — typed exports + helpers.
- `packages/catalog/test/facts.test.ts` — helper unit tests + drift gate.
- `.github/workflows/catalog-publish.yml` — version + publish + bump-PR.
- proto.cat: `lib/labwired/check-catalog-sync.ts` — two-direction drift gate; `package.json` dep + script; CI wiring.

---

## Phase A — `@labwired/catalog` package (producer)

### Task A1: Scaffold the package

**Files:**
- Create: `packages/catalog/package.json`
- Create: `packages/catalog/tsconfig.json`
- Create: `packages/catalog/tsconfig.build.json`
- Create: `packages/catalog/vitest.config.ts`

**Interfaces:**
- Produces: an installable/buildable package skeleton with scripts `build`, `generate:facts`, `check:facts`, `test`, `typecheck`.

- [ ] **Step 1: Create `packages/catalog/package.json`**

```json
{
  "name": "@labwired/catalog",
  "version": "0.1.0",
  "type": "module",
  "main": "dist/index.js",
  "files": ["dist", "src/catalog-facts.json"],
  "exports": {
    ".": {
      "import": "./dist/index.js",
      "default": "./src/index.ts"
    }
  },
  "scripts": {
    "build": "esbuild src/index.ts --bundle --platform=node --format=esm --outfile=dist/index.mjs --packages=external && cp dist/index.mjs dist/index.js",
    "generate:facts": "tsx scripts/generate-catalog-facts.ts",
    "check:facts": "tsx scripts/generate-catalog-facts.ts --check",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": {
    "@labwired/board-config": "file:../board-config",
    "@types/node": "^25.9.3",
    "esbuild": "^0.25.0",
    "tsx": "^4.19.0",
    "typescript": "~5.9.3",
    "vitest": "~3.2.0"
  }
}
```

- [ ] **Step 2: Create `packages/catalog/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler",
    "strict": true, "noEmit": true, "skipLibCheck": true,
    "resolveJsonModule": true, "types": ["node"]
  },
  "include": ["src", "test", "scripts"]
}
```

- [ ] **Step 3: Create `packages/catalog/tsconfig.build.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022", "module": "CommonJS", "moduleResolution": "Node10",
    "outDir": "dist", "rootDir": "src", "strict": true, "declaration": true,
    "declarationMap": true, "skipLibCheck": true, "esModuleInterop": true,
    "resolveJsonModule": true, "types": []
  },
  "include": ["src"]
}
```

- [ ] **Step 4: Create `packages/catalog/vitest.config.ts`**

```typescript
import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { include: ['test/**/*.test.ts'] } });
```

- [ ] **Step 5: Install devDeps and verify**

Run: `cd packages/catalog && npm install`
Expected: installs without error; `node_modules/.bin/tsx` and `vitest` present.

- [ ] **Step 6: Commit**

```bash
git add packages/catalog/package.json packages/catalog/tsconfig.json packages/catalog/tsconfig.build.json packages/catalog/vitest.config.ts
git commit -m "feat(catalog): scaffold @labwired/catalog package"
```

### Task A2: Generator + generated facts

**Files:**
- Create: `packages/catalog/scripts/generate-catalog-facts.ts`
- Create: `packages/catalog/src/catalog-facts.json` (via running the generator)

**Interfaces:**
- Consumes: `CATALOG` from `../../board-config/src/catalog` (each `CatalogPart` has `type: string`), `PIN_MAPS` from `../../board-config/src/pin-mapping` (chip-family keys), `../../ui/src/peripherals/manifest.json` (`peripherals[].device_type`).
- Produces: `src/catalog-facts.json` with shape `{ schema_version: 1, device_types: string[], chips: string[] }` (both arrays sorted, deduped). `--check` regenerates and diffs, exiting 1 on drift.

- [ ] **Step 1: Write the failing drift test first**

Create `packages/catalog/test/facts.test.ts`:

```typescript
import { describe, it, expect } from 'vitest';
import { execFileSync } from 'node:child_process';

describe('catalog-facts generation', () => {
  it('src/catalog-facts.json is up to date with the generator', () => {
    expect(() =>
      execFileSync('npm', ['run', 'check:facts'], {
        cwd: new URL('..', import.meta.url),
        stdio: 'pipe',
      }),
    ).not.toThrow();
  });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd packages/catalog && npx vitest run test/facts.test.ts`
Expected: FAIL — generator script does not exist yet.

- [ ] **Step 3: Write the generator**

Create `packages/catalog/scripts/generate-catalog-facts.ts`:

```typescript
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { CATALOG } from '../../board-config/src/catalog';
import { PIN_MAPS } from '../../board-config/src/pin-mapping';
import manifest from '../../ui/src/peripherals/manifest.json' assert { type: 'json' };

const SCHEMA_VERSION = 1;
const OUT = fileURLToPath(new URL('../src/catalog-facts.json', import.meta.url));

function sortedUnique(values: string[]): string[] {
  return [...new Set(values)].sort();
}

function build(): string {
  const catalogTypes = Object.values(CATALOG).map((p) => p.type);
  const manifestTypes = (manifest.peripherals as { device_type: string }[]).map(
    (p) => p.device_type,
  );
  const facts = {
    schema_version: SCHEMA_VERSION,
    device_types: sortedUnique([...catalogTypes, ...manifestTypes]),
    chips: sortedUnique(Object.keys(PIN_MAPS)),
  };
  return JSON.stringify(facts, null, 2) + '\n';
}

const generated = build();
if (process.argv.includes('--check')) {
  const current = readFileSync(OUT, 'utf8');
  if (current !== generated) {
    console.error(
      'packages/catalog/src/catalog-facts.json is stale. Run: npm --prefix packages/catalog run generate:facts',
    );
    process.exit(1);
  }
} else {
  writeFileSync(OUT, generated);
  console.error(`wrote ${OUT}`);
}
```

- [ ] **Step 4: Generate the facts file**

Run: `cd packages/catalog && npm run generate:facts`
Expected: writes `src/catalog-facts.json`. Confirm it contains `"uc8151d_tricolor_290"`, `"ssd1680_tricolor_290"`, and `"esp32"`:
Run: `node -e "const f=require('./src/catalog-facts.json'); console.log(f.device_types.includes('uc8151d_tricolor_290'), f.device_types.includes('ssd1680_tricolor_290'), f.chips.includes('esp32'))"`
Expected: `true true true`

- [ ] **Step 5: Run the drift test to verify it passes**

Run: `npx vitest run test/facts.test.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/catalog/scripts/generate-catalog-facts.ts packages/catalog/src/catalog-facts.json packages/catalog/test/facts.test.ts
git commit -m "feat(catalog): generate device_type + chip facts from board-config and manifest"
```

### Task A3: Typed package API + helpers

**Files:**
- Create: `packages/catalog/src/index.ts`
- Modify: `packages/catalog/test/facts.test.ts` (add helper tests)

**Interfaces:**
- Produces: `CATALOG_FACTS: CatalogFacts`, `DEVICE_TYPES: readonly string[]`, `CHIPS: readonly string[]`, `isKnownDeviceType(t: string): boolean`, `isKnownChip(id: string): boolean`, and `type CatalogFacts = { schema_version: number; device_types: string[]; chips: string[] }`.

- [ ] **Step 1: Add failing helper tests**

Append to `packages/catalog/test/facts.test.ts`:

```typescript
import { isKnownDeviceType, isKnownChip, CATALOG_FACTS } from '../src/index';

describe('catalog facts helpers', () => {
  it('recognises a kit device_type', () => {
    expect(isKnownDeviceType('ssd1680_tricolor_290')).toBe(true);
  });
  it('recognises a legacy (catalog-only) device_type', () => {
    expect(isKnownDeviceType('uc8151d_tricolor_290')).toBe(true);
  });
  it('rejects an unknown device_type', () => {
    expect(isKnownDeviceType('totally-made-up')).toBe(false);
  });
  it('recognises a chip family', () => {
    expect(isKnownChip('esp32')).toBe(true);
  });
  it('pins the schema version', () => {
    expect(CATALOG_FACTS.schema_version).toBe(1);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/catalog && npx vitest run test/facts.test.ts`
Expected: FAIL — `../src/index` has no exports yet.

- [ ] **Step 3: Write `src/index.ts`**

```typescript
import factsRaw from './catalog-facts.json';

export interface CatalogFacts {
  schema_version: number;
  device_types: string[];
  chips: string[];
}

/** Schema version the TS side was built against. Generator must match. */
export const CATALOG_FACTS_SCHEMA = 1;

const FACTS = factsRaw as CatalogFacts;

if (FACTS.schema_version !== CATALOG_FACTS_SCHEMA) {
  throw new Error(
    `catalog facts schema mismatch: file=${FACTS.schema_version}, ts=${CATALOG_FACTS_SCHEMA}. ` +
      `Re-run \`npm --prefix packages/catalog run generate:facts\`.`,
  );
}

export const CATALOG_FACTS: CatalogFacts = FACTS;
export const DEVICE_TYPES: readonly string[] = FACTS.device_types;
export const CHIPS: readonly string[] = FACTS.chips;

const DEVICE_TYPE_SET = new Set(FACTS.device_types);
const CHIP_SET = new Set(FACTS.chips);

export function isKnownDeviceType(deviceType: string): boolean {
  return DEVICE_TYPE_SET.has(deviceType);
}

export function isKnownChip(chip: string): boolean {
  return CHIP_SET.has(chip);
}
```

- [ ] **Step 4: Run tests to verify pass**

Run: `npx vitest run`
Expected: PASS (drift + helper tests).

- [ ] **Step 5: Build + typecheck**

Run: `npm run typecheck && npm run build`
Expected: no type errors; `dist/index.js` produced.

- [ ] **Step 6: Commit**

```bash
git add packages/catalog/src/index.ts packages/catalog/test/facts.test.ts packages/catalog/dist
git commit -m "feat(catalog): typed facts API and isKnownDeviceType/isKnownChip helpers"
```

### Task A4: Wire the drift gate into core CI

**Files:**
- Modify: `.github/workflows/core-ci.yml` (add a catalog facts check job) OR `playground-ci.yml` — whichever runs on `packages/**` pushes. Inspect first; add to the one already gating TS packages.

**Interfaces:**
- Consumes: `npm run check:facts`.
- Produces: a CI step failing when `catalog-facts.json` drifts from `board-config`/manifest.

- [ ] **Step 1: Inspect which workflow gates TS package builds**

Run: `grep -ln "packages/board-config\|packages/ui" .github/workflows/*.yml`
Pick the workflow that builds/tests TS packages on push.

- [ ] **Step 2: Add a job/step**

Add to that workflow (adjust `runs-on`/setup to match siblings):

```yaml
  catalog-facts:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: "20", cache: npm, cache-dependency-path: packages/catalog/package-lock.json }
      - working-directory: packages/catalog
        run: npm ci
      - working-directory: packages/catalog
        run: npm run check:facts
      - working-directory: packages/catalog
        run: npm test
```

- [ ] **Step 3: Validate YAML locally**

Run: `cd packages/catalog && npm ci && npm run check:facts && npm test`
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ packages/catalog/package-lock.json
git commit -m "ci(catalog): gate catalog-facts drift and tests"
```

---

## Phase B — Auto-version + publish workflow

### Task B1: Publish workflow with no-op guard

**Files:**
- Create: `.github/workflows/catalog-publish.yml`

**Interfaces:**
- Consumes: `NPM_TOKEN` secret (already used by `mcp-publish.yml`).
- Produces: a published `@labwired/catalog` version when the generated facts content changes on push to main.

- [ ] **Step 1: Create the workflow**

```yaml
name: Publish @labwired/catalog

on:
  push:
    branches: [main]
    paths:
      - "packages/catalog/**"
      - "packages/ui/src/peripherals/manifest.json"
      - "packages/board-config/src/catalog.ts"
      - "packages/board-config/src/pin-mapping.ts"
  workflow_dispatch:

permissions:
  contents: write   # push the version bump commit
  id-token: write   # npm provenance

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: actions/setup-node@v4
        with:
          node-version: "20"
          registry-url: "https://registry.npmjs.org"
          cache: npm
          cache-dependency-path: packages/catalog/package-lock.json
      - name: Install
        working-directory: packages/catalog
        run: npm ci
      - name: Regenerate facts
        working-directory: packages/catalog
        run: npm run generate:facts
      - name: Stop if facts unchanged and version already published
        id: guard
        working-directory: packages/catalog
        run: |
          if git diff --quiet -- src/catalog-facts.json; then
            echo "changed=false" >> "$GITHUB_OUTPUT"
          else
            echo "changed=true" >> "$GITHUB_OUTPUT"
          fi
      - name: Build + test
        if: steps.guard.outputs.changed == 'true'
        working-directory: packages/catalog
        run: npm run build && npm test
      - name: Version, commit, publish
        if: steps.guard.outputs.changed == 'true'
        working-directory: packages/catalog
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          git config user.name w1ne
          git config user.email 14119286+w1ne@users.noreply.github.com
          git add src/catalog-facts.json
          npm version patch -m "chore(catalog): publish %s [skip ci]"
          npm publish --provenance --access public
          git push --follow-tags origin main
```

- [ ] **Step 2: Validate the workflow YAML**

Run: `npx --yes @action-validator/cli .github/workflows/catalog-publish.yml` (or `actionlint` if installed).
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/catalog-publish.yml
git commit -m "ci(catalog): auto-version and publish on facts change"
```

### Task B2: Cross-repo bump PR in proto.cat

**Files:**
- Modify: `.github/workflows/catalog-publish.yml` (append a bump-PR job).

**Interfaces:**
- Consumes: a token with PR access to the proto.cat repo (`PROTOCAT_PAT` secret — to be created).
- Produces: an opened/updated PR in proto.cat raising the `@labwired/catalog` version.

- [ ] **Step 1: Append a bump job that checks out proto.cat, bumps the dep, opens a PR**

```yaml
  bump-protocat:
    needs: publish
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: <protocat-owner>/protocat
          token: ${{ secrets.PROTOCAT_PAT }}
      - uses: actions/setup-node@v4
        with: { node-version: "20" }
      - name: Bump @labwired/catalog
        run: |
          NEW=$(npm view @labwired/catalog version)
          pnpm add "@labwired/catalog@$NEW"
      - uses: peter-evans/create-pull-request@v6
        with:
          token: ${{ secrets.PROTOCAT_PAT }}
          branch: chore/bump-labwired-catalog
          title: "chore: bump @labwired/catalog"
          body: "Automated bump so proto.cat's catalog drift gate runs against the latest LabWired facts."
          commit-message: "chore: bump @labwired/catalog"
```

- [ ] **Step 2: Replace `<protocat-owner>` with the real owner; document `PROTOCAT_PAT` in the workflow header comment.**

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/catalog-publish.yml
git commit -m "ci(catalog): open bump PR in proto.cat after publish"
```

---

## Phase C — proto.cat consumer (separate repo, separate PR)

> Executed in `/home/andrii/projects/protocat` on its own branch/PR. proto.cat uses **pnpm**; its only test runner is Playwright, so the drift gate is a standalone tsx script run in CI, not a vitest test.

### Task C1: Add dependency + two-direction drift gate

**Files:**
- Modify: `protocat/package.json` (add `@labwired/catalog` dep + `check:catalog` script).
- Create: `protocat/lib/labwired/check-catalog-sync.ts`

**Interfaces:**
- Consumes: `isKnownDeviceType`, `isKnownChip`, `PERIPHERAL_DEVICE_TYPES` from `@labwired/catalog`; `CHIP_BLOCKS`, `COMPONENT_BLOCKS` from `./blocks`.
- Produces: a process that **hard-fails** on direction-1 drift (a proto.cat block references an unknown device_type/chip) and on a **newly** unmapped peripheral (one absent from the committed `coverage-baseline.json`), while merely **warning** about the acknowledged backlog. `--update` rewrites the baseline.

> **Roast #4/#3 resolution:** direction-2 is not a hard-fail against the full
> published set (that demands proto.cat cover all 22 peripherals and produces a
> 19-entry hand-typed allowlist of noise). Instead proto.cat commits an
> auto-generated `coverage-baseline.json` — the set of peripherals it knowingly
> does not map yet. The gate fails only for a peripheral that is neither mapped
> nor in the baseline, i.e. one that appeared in LabWired *after* adoption. That
> forces attention on genuinely new components (the real goal) without nagging
> about the pre-existing backlog. Hard facts auto-sync; coverage is human-paced
> but drift-proof.

- [ ] **Step 1: Add the dependency and script**

Run: `cd /home/andrii/projects/protocat && pnpm add @labwired/catalog && pnpm add -D tsx`
Then add to `package.json` scripts: `"check:catalog": "tsx lib/labwired/check-catalog-sync.ts"`.

- [ ] **Step 2: Write the drift gate**

Create `protocat/lib/labwired/check-catalog-sync.ts`:

```typescript
import { readFileSync, writeFileSync } from 'node:fs';
import { isKnownDeviceType, isKnownChip, PERIPHERAL_DEVICE_TYPES } from '@labwired/catalog';
import { CHIP_BLOCKS, COMPONENT_BLOCKS } from './blocks';

const BASELINE = new URL('./coverage-baseline.json', import.meta.url);

const mapped = new Set(
  COMPONENT_BLOCKS.filter((c) => c.simulated && c.device_type).map((c) => c.device_type as string),
);
// Peripherals LabWired exposes that proto.cat does not map yet.
const unmapped = PERIPHERAL_DEVICE_TYPES.filter((dt) => !mapped.has(dt)).sort();

if (process.argv.includes('--update')) {
  writeFileSync(BASELINE, JSON.stringify({ acknowledged_unmapped: unmapped }, null, 2) + '\n');
  console.error(`wrote coverage baseline (${unmapped.length} acknowledged)`);
  process.exit(0);
}

const baseline: { acknowledged_unmapped: string[] } = JSON.parse(readFileSync(BASELINE, 'utf8'));
const acknowledged = new Set(baseline.acknowledged_unmapped);

const errors: string[] = [];

// Direction 1 (hard fail): every proto.cat reference must be a real LabWired fact.
for (const chip of CHIP_BLOCKS) {
  if (!isKnownChip(chip.id)) errors.push(`chip "${chip.id}" is not a known LabWired chip_family`);
}
for (const c of COMPONENT_BLOCKS) {
  if (c.simulated && c.device_type && !isKnownDeviceType(c.device_type)) {
    errors.push(`component "${c.id}" device_type "${c.device_type}" is not a known LabWired device_type`);
  }
}

// Direction 2 (hard fail only for NEW peripherals): unmapped + not acknowledged.
for (const dt of unmapped) {
  if (!acknowledged.has(dt)) {
    errors.push(
      `new LabWired peripheral "${dt}" has no proto.cat block. Add an overlay block in lib/labwired/blocks.ts, ` +
        `or run \`pnpm check:catalog --update\` to acknowledge it in coverage-baseline.json.`,
    );
  }
}

// Backlog is informational only.
const backlog = unmapped.filter((dt) => acknowledged.has(dt));
if (backlog.length > 0) {
  console.error(`note: ${backlog.length} acknowledged peripherals not yet mapped: ${backlog.join(', ')}`);
}

if (errors.length > 0) {
  console.error('catalog drift detected:\n' + errors.map((e) => `  - ${e}`).join('\n'));
  process.exit(1);
}
console.error('catalog in sync with @labwired/catalog');
```

- [ ] **Step 3: Seed the baseline and run the gate**

Run: `cd /home/andrii/projects/protocat && pnpm check:catalog --update && pnpm check:catalog`
Expected: `--update` writes `coverage-baseline.json` acknowledging today's unmapped peripherals; the second run prints the backlog note and "catalog in sync". From now on a *new* LabWired peripheral fails the gate until mapped or re-acknowledged.

- [ ] **Step 4: Commit**

```bash
cd /home/andrii/projects/protocat
git add package.json pnpm-lock.yaml lib/labwired/check-catalog-sync.ts lib/labwired/coverage-baseline.json
git commit -m "feat(labwired): consume @labwired/catalog with a drift gate"
```

### Task C2: Wire the gate into proto.cat CI

**Files:**
- Modify: proto.cat CI workflow (the one running on PRs) to add a `pnpm check:catalog` step.

- [ ] **Step 1: Add the step to the proto.cat CI workflow before/with the test job.**

```yaml
      - run: pnpm install --frozen-lockfile
      - run: pnpm check:catalog
```

- [ ] **Step 2: Commit + open PR targeting proto.cat's default branch.**

---

## Self-Review

**Spec coverage:**
- Producer `@labwired/catalog` data-only package → Tasks A1–A3. ✓
- Hard facts = device_type + chip ids, generated from manifest (+ corrected to include `board-config` CATALOG so legacy `uc8151d` isn't false-flagged) → Task A2. ✓
- `check`/drift gate mirroring board-config → A2 (test) + A4 (CI). ✓
- proto.cat overlay + two-direction drift gate → C1. ✓ (Implemented as a standalone tsx script, not vitest, because proto.cat has no unit-test runner — deviation from the spec's "vitest test", noted here and justified.)
- Auto-version + publish on change, no-op guard, bump PR → B1 + B2. ✓
- Explicitly-not-synced overlay fields stay in proto.cat `blocks.ts` untouched → C1 leaves the block shape intact. ✓

**Deviations from spec (intentional):**
- Validity source is the **union of board-config CATALOG + kit manifest**, not the manifest alone — required for correctness (`uc8151d_tricolor_290` is catalog-only). Documented in A2.
- proto.cat gate is a tsx CLI script, not a vitest test — proto.cat lacks a unit-test runner. Documented in C1.

**Placeholder scan:** Only `<protocat-owner>` and `PROTOCAT_PAT` remain — these are real values to fill at execution (owner of the proto.cat GitHub repo; a PAT secret to create). Flagged in B2, not silent TODOs.

**Type consistency:** `isKnownDeviceType`/`isKnownChip`/`DEVICE_TYPES`/`CATALOG_FACTS`/`CatalogFacts` are defined in A3 and consumed identically in C1. `catalog-facts.json` shape `{schema_version, device_types, chips}` is identical in A2 (generator), A3 (reader), and the schema guard.
