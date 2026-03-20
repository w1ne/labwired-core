# Release Readiness Checklist (v0.13.0)

**Date**: 2026-03-20
**Version**: 0.13.0
**Coordinator**: @w1ne

## 1. Documentation Audit
- [x] **Changelog Updated**: `CHANGELOG.md` updated in both root and core with all changes since v0.12.1.
- [x] **Release Notes**: GitHub release created with full changelog body.
- [x] **Process Docs**: `RELEASE_PROCESS.md` remains current.

## 2. Codebase Integrity
- [x] **Tests Passing**: `cargo test --workspace` — 144 lib tests pass; integration tests requiring cross-compiled firmware skipped (expected on dev host).
- [x] **Lints Passing**: `cargo clippy --workspace -- -D warnings` — clean.
- [x] **Formatting**: `cargo fmt --all -- --check` — clean.
- [x] **Version Bump**: `Cargo.toml` updated to `0.13.0`; `cargo check` confirms all crates resolve to `v0.13.0`.
- [x] **Go Backend**: `go test ./cmd/... ./internal/...` passes in CI; `go build` produces clean binary.

## 3. Artifacts & Packaging
- [x] **Core Tag**: `v0.13.0` pushed to `labwired-core`.
- [x] **Root Tag**: `v0.13.0` pushed to `labwired`.
- [x] **GitHub Release**: Published at https://github.com/w1ne/labwired/releases/tag/v0.13.0.
- [x] **Submodule Pointer**: Root repo `core` submodule updated to v0.13.0 merge commit.

## 4. Workflow & API Audit
- [x] **api-ci.yml**: Fast Go tests on push/PR — no issues.
- [x] **foundry-ci.yml**: Backend + frontend build and smoke test on port 18080 — no issues.
- [x] **foundry-deploy.yml**: Build→push→deploy with health gate validation and automatic rollback — no issues.
- [x] **pluto-maintenance.yml**: Scheduled cleanup + emergency repair with Stripe fallback — no issues.
- [x] **core-ci.yml**: Rust integration smoke with portable Zig C-compiler fallback — no issues.
- [x] **Shell Scripts**: Hardcoded Go binary paths replaced with portable `command -v go` fallback in all 4 demo/test scripts.

## 5. Final Review
- [x] **CI Green**: All GitHub Actions workflows passing on `main`.
- [x] **No Merge Conflicts**: Working tree clean; core submodule on tagged commit.

---
**Status**: [x] READY FOR RELEASE

## Known Issues (v0.13.0)
- `demo_blinky` and `strict_onboarding` integration tests require cross-compiled ARM firmware; linker script fix pending for RP2040 target.
- `STRIPE_PAYMENT_LINK` in frontend contains placeholder value — must be replaced before enabling live payments.
- Clerk authentication is optional on the backend; account routes degrade gracefully when `CLERK_SECRET_KEY` is unset.
