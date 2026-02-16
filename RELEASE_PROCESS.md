# LabWired Unified Release Process

This is the release runbook for the entire monorepo (`core`, `vscode`, `ai`, `docs`).
It supports scoped releases, but enforces cross-component compatibility because the parts are coupled.

Execution authority:

1. This file is the operational source of truth for release execution.
2. `core/docs/release_strategy.md` defines policy and branching rationale.

## 1. Release Types

1. Platform release:
   - scope: `core + vscode + ai + docs`
2. Scoped release:
   - scope: one or more components (for example `core` only, `vscode` only)
   - still requires baseline cross-component checks in Section 4

## 2. Roles and Ownership

1. Release owner:
   - defines scope
   - runs gates
   - blocks release on any red gate
2. Component reviewer:
   - one reviewer per in-scope component
3. Final approver:
   - confirms evidence pack and signs off publish

## 3. Branching and Versioning

Run from repo root:

```bash
git checkout develop
git pull
git checkout -b release/vX.Y.Z
```

Version/changelog policy:

1. Always update root `CHANGELOG.md`.
2. If `core` is in scope, update `core/Cargo.toml` workspace version and `core/CHANGELOG.md`.
3. If `vscode` is in scope, update `vscode/package.json`.
4. Any intentional version skew must be documented in release notes.
5. Release scope must be declared in the PR description as: `core|vscode|ai|docs`.
6. Documentation updates are required for every in-scope component:
   - `core`: update `core/docs/*` and/or `core/README.md` when behavior, commands, or configs change.
   - `vscode`: update `vscode/README.md` and related extension docs when commands, UX, or compatibility changes.
   - `ai`: update `ai/README.md` and `ai/docs/*` when flows, prompts, or outputs change.
   - `docs`: update impacted files in `docs/*`.
7. If a component is in scope and no docs change is needed, add explicit justification in the release PR under `Docs Impact`.

## 4. Global Required Gates (All Releases)

These gates run for every release, even scoped releases.

### 4.1 Core workspace safety

Run from `core/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

### 4.2 Core runtime compatibility

Run from `core/`:

```bash
cargo build -p demo-blinky --release --target thumbv7m-none-eabi
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/demo-blinky \
  --system configs/systems/ci-fixture-uart1.yaml \
  --max-steps 20000 \
  --out-dir out/unsupported-audit/ci-fixture \
  --fail-on-unsupported
cargo run -q -p labwired-cli -- test --script examples/ci/uart-ok.yaml --no-uart-stdout
```

### 4.3 Cross-component compatibility baseline

Even when releasing one part, verify integration points are not broken:

1. `core` binary surfaces still build:

```bash
(cd core && cargo build -p labwired-cli --release && cargo build -p labwired-dap --release)
```

2. VS Code extension compiles against current repo state:

```bash
(cd vscode && npm ci && npm run compile)
```

3. AI dry-run smoke stays healthy:

```bash
python3 ai/tests/demo_dry_run.py --mode fallback --device LM75B --docker
```

## 5. Component-Specific Gates (Run for In-Scope Components)

### 5.1 Core scope gates

Run from `core/`:

```bash
cargo test -p labwired-core --test strict_onboarding -- --nocapture
cargo build -p firmware-ci-fixture --release --target thumbv6m-none-eabi
cargo build -p riscv-ci-fixture --release --target riscv32imac-unknown-none-elf
cargo build -p firmware-h563-io-demo --release --target thumbv7m-none-eabi
cargo build -p firmware-h563-fullchip-demo --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/ci/riscv-uart-ok.yaml --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/io-smoke.yaml --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/fullchip-smoke.yaml --no-uart-stdout
python3 -m pip install -r requirements.txt
mkdocs build
```

### 5.2 VS Code scope gates

Run from `vscode/`:

```bash
npm ci
npm run compile
npm test
```

Also run from `core/`:

```bash
cargo build -p labwired-dap --release
```

### 5.3 AI scope gates

Run from repo root:

```bash
python3 ai/tests/demo_dry_run.py --mode fallback --device LM75B --docker
```

If AI changes affect generated config/firmware consumed by core, re-run Section 5.1 smoke tests.

### 5.4 Docs scope gates

Run from `core/`:

```bash
python3 -m pip install -r requirements.txt
mkdocs build
```

If docs include commands/examples, commands must execute as written.

## 6. CI Gate Policy

Required workflows must be green before tagging:

1. `.github/workflows/core-ci.yml`
2. `.github/workflows/vscode-ci.yml`

No bypasses:

1. Do not tag with pending required checks.
2. Do not publish based only on local partial checks.

## 7. Evidence Pack (Mandatory)

Attach to release PR/tracking issue:

1. Scope declaration (`core|vscode|ai|docs`).
2. Exact commands executed.
3. Key outputs:
   - unsupported-instruction audit result and artifact path
   - representative smoke output (UART or equivalent deterministic signal)
4. CI run URLs.
5. Known issues and risk acceptance (if any).
6. Docs impact summary for each in-scope component with changed file paths.

## 8. Release Notes Standard

Use `.github/RELEASE_TEMPLATE.md` for every GitHub release draft.
Do not publish with missing template fields.

Minimum required content:

1. Scope and commit SHA.
2. Validation evidence (local + CI + instruction audit).
3. Breaking changes (`None` if not applicable).
4. Known issues (`None` if none).
5. Documentation updates by component (or explicit `No docs changes` with reason).

## 9. Publish Procedure

After release PR merge to `main`:

```bash
git checkout main
git pull
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

Then:

1. Create and publish a GitHub release for tag `vX.Y.Z` (annotated tag must already be pushed).
2. Use `.github/RELEASE_TEMPLATE.md` for the release description and fill all required fields.
3. Title format: `Release vX.Y.Z: <short summary>`.
4. Description must include scope, validation evidence, CI links, breaking changes, and known issues.
5. Description must include documentation updates for each in-scope component.
6. Attach artifacts for in-scope components.
7. Merge `main` back into `develop`.

## 10. Hotfix Procedure

For production regressions only:

```bash
git checkout -b hotfix/vX.Y.Z+1 main
```

Run all global gates (Section 4) plus affected component gates (Section 5), then merge/tag/back-merge.

## 11. Hard Stop Conditions

Release is blocked if any condition is true:

1. Any required gate fails.
2. Unsupported instruction audit reports non-zero unsupported instructions.
3. Required CI workflows are not green.
4. Evidence pack is incomplete.
5. Release notes do not follow `.github/RELEASE_TEMPLATE.md`.
