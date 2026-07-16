# Unified Board/Example CI Matrix — Design

Date: 2026-06-28
Status: Approved (brainstorm), pending implementation plan
Repo: `labwired-core` (public)

## Problem

Board/example CI in `labwired-core` is sprawling and inconsistent:

- Four hand-written, near-duplicate workflow YAMLs each repeat the same shape
  (install Rust → install toolchain → fetch a CMSIS/Cube pack → build firmware →
  run a test suite): `core-board-ci-fixture-arm.yml`, `core-board-ci-fixture-riscv.yml`,
  `core-board-nucleo-h563zi.yml`, `core-iolink-station.yml`.
- They are not even consistent: the first three are **nightly** (cron + dispatch);
  only `core-iolink-station` runs on every push/PR to `main` and is a blocking gate.
- `core-iolink-station.yml` is tied to a single *example*, not a generic board.
- This does not scale: 200 boards would mean 200 hand-written YAMLs.
- The gate recently went red because the `master-fw` Makefile depends on
  `third_party/iolinki/src/frame.c`, which existed only in an unpushed local iolinki
  commit. (Resolved out-of-band by PR #393, which bumps the iolinki submodule to
  v1.1.3.)

Two adjacent subsystems already do config-driven board validation and should be
folded into one mechanism:

- `scripts/validate_hw_targets.py` over `configs/onboarding/*` — per-system sim
  validation that writes metadata back into manifests (run by
  `core-validate-hw-targets.yml`).
- `scripts/generate_tier1_scoreboard.py` and `scripts/generate_validation_status.py`
  — aggregate generators that produce scoreboards/status pages.

## Constraints

1. **Core gates are public proof.** `labwired-core` is public; its green board gates
   are the public evidence that real firmware verifiably runs. Gate jobs must stay in
   core, stay green, and **keep their existing check names** so branch-protection
   required-checks and any external evidence links do not break.
2. **No app↔core test duplication.** The private app repo's `core-ci.yml`
   ("Core Integration CI") currently re-scripts core's `integration-smoke` +
   `determinism` tests against the pinned core submodule. The test *definitions* must
   live once (in core); the app should *invoke* them, not re-implement them.
3. Each migration phase must be independently shippable and verifiable. Old YAML is
   deleted only after its replacement entry is proven green.

## Approach (chosen: Hybrid)

Thin central policy manifest + co-located execution scripts.

- A small central manifest holds **policy/metadata only** per entry.
- The **heavy build/test/setup logic lives next to the board** in co-located scripts.
- A single workflow reads the manifest, filters by event, fans out a matrix, and runs
  an aggregation pass afterward.

Rejected alternatives:
- *Central declarative manifest + generic runner* — forces genuinely heterogeneous
  build steps (Cube-pack fetch, recursive submodules, 3-ELF multichip build) into
  declarative YAML; the manifest becomes a second scripting language.
- *Pure co-located convention + auto-discovery* — scales well but loses the
  single-glance "what runs / what gates" view and does not model aggregate generators.

## Components

### 1. Manifest — `configs/ci/boards.yml`

Single source of truth. One entry per target; policy + metadata only:

```yaml
- id: iolink-station-l476        # also the CI check name
  kind: firmware-gate            # firmware-gate | sim-validate | aggregate
  path: examples/iolink-station  # where ci/ scripts live
  toolchains: [arm-none-eabi]    # toolchains the workflow installs
  packs: [stm32cubel4@v1.18.2]   # named, reusable pack fetchers
  submodules: recursive          # checkout submodule mode
  gate: true                     # true → PR/push to main; false → nightly only
```

`kind` selects runner behavior:
- `firmware-gate` — run `<path>/ci/build.sh` then `<path>/ci/test.sh`.
- `sim-validate` — onboarding-style: build labwired bin, run sim against the entry's
  system config(s); wraps `validate_hw_targets.py`.
- `aggregate` — run once after fan-out, consuming matrix artifacts (tier1 scoreboard,
  validation status).

### 2. Co-located execution

Per-board logic moves out of YAML into `<path>/ci/build.sh` and `<path>/ci/test.sh`.
For `iolink-station`, that means the 3-ELF multichip build, the Cube-pack wiring, and
`LABWIRED_REQUIRE_IOLINK_ELFS=1` live in `examples/iolink-station/ci/`. New board =
new dir + one manifest row; the central workflow is untouched.

### 3. Workflow — `core-board-ci.yml`

- **setup job**: parse + *validate* the manifest (every `path`/script exists → fail
  fast), filter by event (PR/push → `gate: true` only; nightly cron → all), emit
  matrix JSON.
- **board job**: `strategy.matrix` from setup. Checkout (submodules per entry) →
  install declared toolchains → fetch declared packs (composite action, §4) → run the
  entry's scripts by `kind` → upload artifacts. Each job's `name` is the entry `id`.
- **aggregate job** (`needs: board`): run `kind: aggregate` entries consuming matrix
  artifacts → publish to step summary (and commit where the existing generators do).

### 4. Reusable pack fetcher — `.github/actions/fetch-pack`

The sparse STM32CubeL4 clone currently inlined in `core-iolink-station.yml` becomes a
composite action keyed by name (e.g. `stm32cubel4@v1.18.2`). Removes the copy-paste;
new packs add a case.

### 5. Public-proof preservation

`firmware-gate` entries run on push/PR to `main`. Migrated jobs keep their existing
check names (notably `iolink-station-l476`) so required-checks and evidence URLs
survive the migration.

### 6. App↔core dedup

The app's `core-ci.yml` stops defining core test steps. It checks out the pinned core
submodule and calls core's own runner with a cheap subset
(`run-board-ci.sh --subset=pin-integrity` = integration-smoke + determinism only, not
the full firmware matrix). Definitions live once in core; the app gets pin-integrity
that auto-tracks new core boards.

## Migration phases

- **Phase 0 — unblock main (DONE via PR #393).** Resolve the `frame.c` missing-source
  bug by bumping the iolinki submodule to v1.1.3. Independent of the redesign.
- **Phase 1 — build the mechanism.** Add manifest schema + `setup`/`board` jobs +
  runner + `fetch-pack` action. Migrate **only** `iolink-station` onto it; keep the
  `iolink-station-l476` check name; prove green. Delete `core-iolink-station.yml`.
- **Phase 2 — nightly firmware boards.** Fold `fixture-arm`, `fixture-riscv`,
  `nucleo-h563zi` into manifest entries (`gate: false`). Delete their YAMLs.
- **Phase 3 — onboarding sim-validate.** Add `kind: sim-validate`; route
  `validate_hw_targets.py` through the runner. Retire `core-validate-hw-targets.yml`.
- **Phase 4 — aggregate generators.** Add `kind: aggregate` entries for the tier1
  scoreboard and validation status.
- **Phase 5 — app dedup.** Rework the app's `core-ci.yml` to invoke
  `run-board-ci.sh --subset=pin-integrity`; delete the duplicated steps.

Each phase ends with the relevant CI green before the superseded YAML is removed.

## Testing

- The `setup` job's manifest validator is itself the first test: a manifest entry
  pointing at a missing path/script must fail the run (no silent skip-to-green).
- Phase 1 acceptance: `iolink-station-l476` check is green via the new workflow with
  the same name, on a PR.
- Per-phase acceptance: the migrated entry is green on a PR before its old YAML is
  deleted; nightly entries verified via `workflow_dispatch`.
