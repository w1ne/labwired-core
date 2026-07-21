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
  - Rename the `[Unreleased]` section header to the new version and date (e.g., `## [X.Y.Z] - YYYY-MM-DD`).
  - Create a new empty `## [Unreleased]` section at the top.
  - Ensure all significant changes from `git log` are captured.
  - Group changes by `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`.

## 2. Release Candidate (RC) verification
- [ ] **Create Release Branch**:
  ```bash
  git checkout -b release/vX.Y.Z
  ```
- [ ] **Commit Version & Changelog**:
  ```bash
  git add Cargo.toml Cargo.lock CHANGELOG.md
  git commit -m "chore: bump version to X.Y.Z"
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
    git tag -a vX.Y.Z -m "Release vX.Y.Z"
    git push origin vX.Y.Z
    ```
- [ ] **Draft Release on GitHub**:
  - Go to [Releases > Draft a new release](https://github.com/w1ne/labwired-core/releases/new).
  - **Tag**: Select `vX.Y.Z`.
  - **Title**: `vX.Y.Z: <Key Highlight/Theme>`
  - **Description**: Copy contents from `.github/RELEASE_TEMPLATE.md` and fill it in with details from `CHANGELOG.md`.

### Artifacts
- [ ] **Release Workflow**: Pushing `vX.Y.Z` triggers
  [`.github/workflows/core-release.yml`](.github/workflows/core-release.yml),
  which builds CLI archives for Linux and macOS targets, uploads them to the
  GitHub Release, and publishes the deployable CI runner image.
- [ ] **Workflow Verification**: Confirm the release workflow completed and the
  expected `labwired-vX.Y.Z-<platform>.tar.gz` assets are attached. Confirm
  GHCR also contains the versioned `ghcr.io/w1ne/labwired:vX.Y.Z` runner
  image. Use that versioned image in CI; the workflow also updates its moving
  convenience tag.
- [ ] **First GHCR publication**: After the first successful image push, open
  the package settings in GitHub Packages and set the GHCR package visibility
  to public. The release smoke job deliberately performs an anonymous pull; if
  the package is still private, it fails with this instruction. Change the
  visibility and re-run the release workflow.
- [ ] **Initial v0.18.0 runner backfill (one time)**: The `v0.18.0` Git tag
  predates the runner-image release workflow. After this workflow reaches
  `main`, run
  [`.github/workflows/core-backfill-runner-image.yml`](.github/workflows/core-backfill-runner-image.yml)
  from Actions with `version` set to `v0.18.0`. It checks out that exact tag,
  publishes only `ghcr.io/w1ne/labwired:v0.18.0` (never `latest`), and runs an
  anonymous pull-and-run smoke test. If the smoke reports a private package,
  make the package public and re-run the backfill workflow. Future release tags
  publish their runner image automatically through `core-release.yml`.
- [ ] **Manual Fallback**: If the release workflow fails, build the CLI locally
  with `cargo build -p labwired-cli --release`, package the `labwired` binary,
  and attach the archive manually with a note in the release description. For
  the runner image, diagnose the failed publish job rather than retagging an
  unverified local image.

## 4. Post-Release
- [ ] **Announce**: Share the release notes on relevant channels (Discord, Twitter, Internal).
