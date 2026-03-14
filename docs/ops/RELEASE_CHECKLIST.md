[← Back to Hub](../README.md)

# LabWired Release Instructions

Use this process for every GitHub release. A release is blocked until all required gates below are green.

## 0. Scope and Ownership

- [ ] Target version selected (`vX.Y.Z`).
- [ ] Release owner assigned.
- [ ] Scope declared: `core`, `vscode`, `ai`, `docs` (one or more).
- [ ] Release branch created from `main`: `release/vX.Y.Z`.

## 1. Version and Changelog Sync

- [ ] `CHANGELOG.md` has a dated entry for `vX.Y.Z`.
- [ ] `core/CHANGELOG.md` has a dated entry for `vX.Y.Z` when `core` is in scope.
- [ ] `core/Cargo.toml` versions are bumped when `core` is in scope.
- [ ] `vscode/package.json` version is bumped when `vscode` is in scope.
- [ ] Cross-component version differences are explicitly called out in release notes.

## 2. Mandatory Local Validation (Pre-Tag)

Run from `core/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo build -p demo-blinky --release --target thumbv7m-none-eabi
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/demo-blinky \
  --system configs/systems/ci-fixture-uart1.yaml \
  --max-steps 20000 \
  --out-dir out/unsupported-audit/ci-fixture \
  --fail-on-unsupported
```

Run from `vscode/` (required when `vscode` is in scope; recommended otherwise):

```bash
npm ci
npm run compile
npm test
```

Run from repo root (required when `ai` is in scope; recommended for platform releases):

```bash
python3 ai/tests/demo_dry_run.py --mode fallback --device LM75B --docker
```

- [ ] All commands above pass for components in release scope.

## 3. Postmortem-Derived Blockers (Do Not Skip)

These checks are mandatory due to incidents on 2026-02-14, 2026-02-15, and 2026-02-16:

- [ ] Unsupported-instruction audit runs with `--fail-on-unsupported` and reports zero unsupported instructions.
- [ ] Full workspace validation is run (`cargo test --workspace` and `cargo build --workspace`), not crate-only validation.
- [ ] Example/smoke validations use fresh release binaries from the current commit.
- [ ] Long-running smoke scenarios are executed with realistic step/cycle limits to avoid false negatives.
- [ ] YAML/config compatibility is validated by running workspace tests before tagging.

## 4. CI Gate Verification

- [ ] Root integration smoke is green: `.github/workflows/core-ci.yml`.
- [ ] Core release-grade workflows are green for the target scope:
  - `core/.github/workflows/core-ci.yml`
  - `core/.github/workflows/core-unsupported-audit.yml`
  - `core/.github/workflows/core-validate-hw-targets.yml` when board/catalog metadata changed
- [ ] `VS Code Extension CI` is green: `.github/workflows/vscode-ci.yml` (required for platform releases and `vscode` scope).
- [ ] No required check is bypassed; no release is cut on pending checks.

## 5. Runtime Evidence Pack (Attach to Release PR/Issue)

- [ ] Core test/build logs saved.
- [ ] Unsupported instruction audit artifacts saved:
  - `core/out/unsupported-audit/<target>/report.md`
  - `core/out/unsupported-audit/<target>/simulator.log`
  - summary TSV files
- [ ] At least one representative smoke output captured (UART or equivalent deterministic signal).
- [ ] CI run links recorded.

## 6. GitHub Release Draft (Standardized Text)

- [ ] Create draft release for tag `vX.Y.Z`.
- [ ] Use `.github/RELEASE_TEMPLATE.md` as the body template.
- [ ] Fill all sections, including:
  - scope
  - validation evidence
  - breaking changes
  - known issues
- [ ] Do not publish until all mandatory gates and sign-offs are complete.

## 7. Publish and Back-Merge

- [ ] Merge `release/vX.Y.Z` into `main`.
- [ ] Tag on `main`: `vX.Y.Z` and push tag.
- [ ] Publish GitHub release.
- [ ] Ensure all release commits are in `main`.

## 8. Sign-Off

- [ ] Engineering sign-off
- [ ] QA sign-off
- [ ] Docs sign-off
- [ ] Release owner approval

## Release Record Template

- Version:
- Release owner:
- Scope:
- Commit SHA:
- Core CI URL:
- VS Code CI URL:
- Unsupported audit report path:
- Smoke evidence path:
- Known issues:
