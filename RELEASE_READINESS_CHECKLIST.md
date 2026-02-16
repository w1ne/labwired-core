# Release Readiness Checklist (v0.12.0)

**Date**: 2026-02-15
**Version**: 0.12.0
**Coordinator**: @w1ne

## 1. Documentation Audit
- [x] **Diataxis Structure Implemented**: Navigation in `mkdocs.yml` follows tutorials/guides/reference/explanation.
- [x] **Key Guides Created**:
    - [x] `release_strategy.md` & `RELEASE_PROCESS.md`
    - [x] `troubleshooting.md`
    - [x] `cli_reference.md` (Verified against `crates/cli`)
    - [x] `configuration_reference.md`
- [x] **Cleanliness**:
    - [x] Removed outdated design docs (`docs/design/`).
    - [x] Checked for broken links (`mkdocs build` passed).

## 2. Codebase Integrity
- [x] **Tests Passing**: `cargo test --workspace` (Unit & Integration).
- [x] **Lints Passing**: `cargo clippy --workspace -- -D warnings`.
- [x] **Formatting**: `cargo fmt --all -- --check`.
- [x] **Feature Flags**: Verified default features build correctly.
- [x] **Fidelity Verification**: `examples/nucleo-h563zi/io-smoke.yaml` passes with `PB0=1`.

## 3. Artifacts & Packaging
- [x] **Version Bump**: `Cargo.toml` updated to `0.12.0`.
- [x] **Changelog**: `CHANGELOG.md` updated with "Documentation Overhaul" and critical `IT` instruction fix.
- [x] **Binaries**: `labwired` CLI builds in release mode (`cargo build --release`).

## 4. Final Review
- [x] **Reviewer Approval**: Verified by @antigravity agent.
- [x] **CI Green**: All GitHub Actions workflows are passing.

---
**Status**: [x] READY FOR RELEASE

## Known Issues (v0.12.0)
- None.
