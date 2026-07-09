# Release Readiness Checklist (v0.18.0)

**Date**: 2026-07-09
**Version**: 0.18.0
**Coordinator**: @w1ne

## 1. Documentation Audit
- [x] **Changelog Updated**: `CHANGELOG.md` captures major changes since `v0.17.10`.
- [x] **Install Docs Updated**: `README.md` pinned install examples reference `v0.18.0`.
- [x] **Process Docs Reviewed**: `RELEASE_PROCESS.md` and release workflow instructions match the tag-triggered automation.
- [x] **Roadmap Updated**: `ROADMAP.md` has a `v0.18.0` section reflecting what shipped, and stale sections are no longer marked `(Current)`.
- [x] **Relative Links Checked**: Release-facing Markdown links in `README.md`, `CHANGELOG.md`, `RELEASE_PROCESS.md`, `RELEASE_READINESS_CHECKLIST.md`, and `ROADMAP.md` resolve locally.

## 2. Codebase Integrity
- [x] **Cargo Check**: `cargo check --workspace` passes after the workspace version bump.
- [x] **Tests Passing**: Workspace tests were verified in split lanes; the monolithic `cargo test --workspace` run was interrupted by the local environment and is recorded below.
- [x] **Formatting**: `cargo fmt --all -- --check` passes.
- [x] **Lints**: `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [x] **Diff Hygiene**: `git diff --check` reports no whitespace errors.

## 3. Artifacts & Packaging
- [x] **Version Bump**: `[workspace.package].version` in root `Cargo.toml` is `0.18.0`; `Cargo.lock` regenerated.
- [x] **Generated Output Cleanup**: No tracked `out/**` run artifacts or accidental build products are part of the release-prep diff.
- [x] **Release Notes Prepared**: `CHANGELOG.md` `0.18.0` section is ready to publish as the GitHub release body.
- [x] **Release Workflow Checked**: `.github/workflows/core-release.yml` creates the release and uploads CLI archives from the pushed tag.
- [x] **Generated UI Manifest Refreshed**: `packages/ui/src/peripherals/manifest.json` in the parent repo was regenerated from the v0.18.0 CLI.

## 4. Final Review
- [x] **Working Tree Reviewed**: Release-prep diff contains expected metadata, documentation, lint fixes, one test-harness fix, and generated lockfile changes.
- [x] **Release Notes Reviewed**: GitHub release body should use the `CHANGELOG.md` `0.18.0` section.
- [ ] **Release Branch Ready**: Release prep is committed on `release/v0.18.0` and ready for PR review.
- [ ] **CI Green**: Pushed release branch passes GitHub Actions.
- [ ] **Release Tagged**: `v0.18.0` tag is created and pushed only after review/CI.

---
**Status**: [x] LOCAL RELEASE PREP COMPLETE; [ ] READY TO TAG

## Known Issues (v0.18.0)
- Full release confidence still depends on the CI matrix for hardware-backed and
  toolchain-specific lanes that are not practical to reproduce in this local
  checkout.
- A monolithic `cargo test --workspace` run reached the long e-paper E2E path
  and was terminated by the local environment. The suite was then verified in
  split lanes:
  - `cargo test -p labwired-core --test intmatrix_alarm`
  - focused core continuation lanes through `xtensa_regs`
  - `cargo test -p labwired-cli --test breakpoints`
  - `cargo test --workspace --exclude labwired-core --exclude labwired-cli --exclude labwired-dap --exclude labwired-gdbstub`
  - `cargo test -p labwired-dap --test e2e` after building `firmware-ci-fixture`
  - `cargo test -p labwired-gdbstub --test gdb_e2e` outside the sandbox because
    the test binds a local TCP port.
