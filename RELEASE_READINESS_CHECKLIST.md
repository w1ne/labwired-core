# Release Readiness Checklist (v0.16.0)

**Date**: 2026-06-18
**Version**: 0.16.0
**Coordinator**: @w1ne

## 1. Documentation Audit
- [ ] **Changelog Updated**: `CHANGELOG.md` captures major changes since the previous release.
- [ ] **Install Docs Updated**: `README.md` pinned install examples reference `v0.16.0`.
- [ ] **Process Docs Updated**: `RELEASE_PROCESS.md` uses neutral version examples.
- [ ] **Roadmap Updated**: `ROADMAP.md` has a `v0.16.0` section reflecting what shipped, and the previous version is no longer marked `(Current)`.

## 2. Codebase Integrity
- [ ] **Cargo Check**: `cargo check --workspace` passes after the workspace version bump.
- [ ] **Tests Passing**: `cargo test --workspace` passes.
- [ ] **Formatting**: `cargo fmt --all -- --check` passes.
- [ ] **Lints**: `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] **Diff Hygiene**: `git diff --check` reports no whitespace errors.

## 3. Artifacts & Packaging
- [ ] **Version Bump**: `[workspace.package].version` in root `Cargo.toml` updated; `Cargo.lock` regenerated.
- [ ] **Generated Output Cleanup**: Tracked `out/**` run artifacts and accidental build products are removed; committed test fixtures remain.
- [ ] **Release Notes Prepared**: `CHANGELOG.md` `0.16.0` section is ready to publish as the GitHub release body.

## 4. Final Review
- [ ] **Working Tree Reviewed**: Release-prep diff contains expected metadata, documentation, generated-output cleanup, formatter changes.
- [ ] **Release Notes Reviewed**: GitHub release body matches the `CHANGELOG.md` `0.16.0` section.

---
**Status**: [ ] READY FOR RELEASE

## Known Issues (v0.16.0)
- `crates/core/tests/e2e_agentdeck_in_sim.rs` remains `#[ignore]`-gated on a 22 MB external firmware ELF that lives outside the repo. Regression assertions are present but only fire when invoked explicitly with `cargo test -- --ignored`.
- The Arduino-ESP32 / FreeRTOS path uses stub mutexes (`return_pd_true`) that unconditionally succeed; a future release should wire real queue semantics. The stub emits a one-time `tracing::warn!` on first call.
