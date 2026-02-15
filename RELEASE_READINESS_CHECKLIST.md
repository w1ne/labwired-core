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
    - [x] Removed "Renode" references from public docs.
    - [x] Checked for broken links (`mkdocs build` passed).

## 2. Codebase Integrity
- [ ] **Tests Passing**: `cargo test --workspace` (Unit & Integration).
- [ ] **Lints Passing**: `cargo clippy --workspace -- -D warnings`.
- [ ] **Formatting**: `cargo fmt --all -- --check`.
- [ ] **Feature Flags**: Verified default features build correctly.

## 3. Artifacts & Packaging
- [x] **Version Bump**: `Cargo.toml` updated to `0.12.0`.
- [ ] **Changelog**: `CHANGELOG.md` updated with "Documentation Overhaul" and other v0.12.0 features.
- [ ] **Binaries**: `labwired` CLI builds in release mode (`cargo build --release`).

## 4. Final Review
- [ ] **Reviewer Approval**: At least one other maintainer has reviewed the `release/v0.12.0` PR.
- [ ] **CI Green**: All GitHub Actions workflows are passing.

---
**Status**: [ ] READY FOR RELEASE
