# LabWired Core Release Process

This document outlines the standardized process for releasing new versions of LabWired Core. Follow this checklist to ensure high-quality, consistent releases.

## 1. Preparation Phase

### git & Codebase
- [ ] **Checkout `main`** and ensure it is up-to-date:
  ```bash
  git checkout main && git pull
  ```
- [ ] **Run Regression Tests**:
  ```bash
  cargo test --workspace
  cargo fmt --all -- --check
  cargo clippy --workspace -- -D warnings
  ```
- [ ] **Verify Documentation**:
  - Run `mkdocs build` to ensure no broken links or config errors.
  - Check that `cli_reference.md` matches the current CLI help output.

### Versioning
- [ ] **Determine Version**: Follow [Semantic Versioning](https://semver.org/).
  - `Update`: Backwards-compatible bug fixes.
  - `Minor`: New features (backwards-compatible).
  - `Major`: Breaking changes.
- [ ] **Bump Version**:
  - Update `version` in `Cargo.toml` (workspace members if necessary).
  - Update `Cargo.lock` by running `cargo check`.

### Changelog
- [ ] **Update `CHANGELOG.md`**:
  - Rename the `[Unreleased]` section header to the new version and date (e.g., `## [0.12.0] - 2026-02-15`).
  - Create a new empty `## [Unreleased]` section at the top.
  - Ensure all significant changes from `git log` are captured.
  - Group changes by `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`.

## 2. Release Candidate (RC) verification
- [ ] **Create Release Branch**:
  ```bash
  git checkout -b release/v0.12.0
  ```
- [ ] **Commit Version & Changelog**:
  ```bash
  git add Cargo.toml Cargo.lock CHANGELOG.md
  git commit -m "chore: bump version to 0.12.0"
  ```
- [ ] **Push & Open PR**:
  - Push the branch and open a PR to `main`.
  - Ensure CI checks pass (Tests, Lint, Build).

## 3. Publication Phase

### GitHub Release
- [ ] **Tag the Release**:
  - After the PR is merged to `main`, tag the commit:
    ```bash
    git checkout main && git pull
    git tag -a v0.12.0 -m "Release v0.12.0"
    git push origin v0.12.0
    ```
- [ ] **Draft Release on GitHub**:
  - Go to [Releases > Draft a new release](https://github.com/w1ne/labwired-core/releases/new).
  - **Tag**: Select `v0.12.0`.
  - **Title**: `v0.12.0: <Key Highlight/Theme>`
  - **Description**: Copy contents from `.github/RELEASE_TEMPLATE.md` and fill it in with details from `CHANGELOG.md`.

### Artifacts (Manual Step until CI is fully automated)
- [ ] **Build Binaries**:
  ```bash
  cargo build --release --bin labwired
  ```
- [ ] **Upload Assets**: Attach the binary (and signature if available) to the GitHub Release.

## 4. Post-Release
- [ ] **Announce**: Share the release notes on relevant channels (Discord, Twitter, Internal).
