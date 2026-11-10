# LabWired Core Release Process

Use this checklist with the [release strategy](docs/release_strategy.md).
Preparation ends with a reviewable release PR and GitHub Release draft.
Publication starts when the verified release tag is pushed.

## 1. Prepare the release branch

- [ ] Fetch `main` and the release tags. Create `release/vX.Y.Z` from the current
  `main` commit. Use a separate worktree if the existing checkout has changes.
  Record the base commit and previous stable core tag. Exclude `firmware-demos-*`
  tags when identifying the previous core release.
- [ ] Review the commits since the stable tag and select the version. Use a patch
  for compatible fixes and a minor version for new capabilities. State API and
  behavior changes explicitly while the project is at `0.x`.
- [ ] Update `[workspace.package].version`, applicable internal package
  references, and `Cargo.lock`. Preserve `version.workspace = true` in member
  manifests. Do not update unrelated dependencies as part of the version bump.
- [ ] Add the dated release section to `CHANGELOG.md` and retain a new
  `Unreleased` section. Group changes by `Added`, `Changed`, `Deprecated`,
  `Removed`, `Fixed`, and `Security` as applicable.
- [ ] Add `docs/releases/vX.Y.Z.md` to Git. Its first line must be
  `# Release vX.Y.Z: <short summary>`. Use the `Features:`, `Improvements:`, and
  optional `Fixes:` structure from the release strategy. Include compatibility
  changes, known limits, and installation instructions. Use absolute URLs for
  links in the release body, including tagged documentation and the compare
  range.
- [ ] Keep the README installation version, current action default, and release
  contract expectations consistent with the prepared version. Preserve
  historical immutable action pins unless their replacement is also verified.

## 2. Validate and review the candidate

Use the pinned Rust toolchain from `rust-toolchain.toml` and the prerequisites in
[core-ci.yml](.github/workflows/core-ci.yml). Start with these checks:

```bash
bash scripts/ci/verify-release-runner-contract.sh
bash scripts/ci/test-install-script.sh
cargo fmt --all -- --check
cargo metadata --locked --no-deps --format-version 1
cargo build -p labwired-cli -p labwired-dap --release --locked
```

- [ ] Run Clippy for the default workspace members and for the additional host
  crates selected by `scripts/ci/host_clippy_scope.py`, using the commands and
  environment from `core-ci.yml`. Do not treat firmware packages as host crates.
- [ ] Run the workspace shards with the same prerequisites as CI, then aggregate
  their reports. The runner applies the checked-in exclusions and issue-linked
  known failures; it also identifies missing reports and unexpected failures.

  ```bash
  mkdir -p out/release-validation
  python3 scripts/ci/workspace_test_shard.py run --shard 1 --report out/release-validation/shard-1.json
  python3 scripts/ci/workspace_test_shard.py run --shard 2 --report out/release-validation/shard-2.json
  python3 scripts/ci/workspace_test_shard.py run --shard 3 --report out/release-validation/shard-3.json
  python3 scripts/ci/workspace_test_aggregate.py --reports out/release-validation
  ```

- [ ] Run the integrity and full-suite jobs from `core-ci.yml`, including their
  firmware builds, `LABWIRED_REQUIRE_FIRMWARE` settings, and feature-specific
  checks. The PR shards exclude some firmware-backed and nightly-only targets;
  their result does not replace the full suite. Confirm that all eight C3
  browser API release tests run: four ROM/OLED tests, three console-route tests
  and the BLE GATT world test. The job verifies the pinned BLE fixture and sets
  `LABWIRED_REQUIRE_C3_BLE=1`, so a missing fixture cannot count as a pass.
  These tests run the browser API natively; the separate `browser-layer` job
  must also build actual WASM and pass its Node regressions. Review board
  coverage and performance results. Keep failures, known failures, and skips
  visible.
- [ ] Run `mkdocs build` in the documented documentation environment. Compare
  `docs/cli_reference.md` with the candidate CLI help output. Check public
  installation examples against the release they name.
- [ ] Commit and push the release branch, then open a PR to `main`. Include the
  candidate SHA, validation results, and any remaining blockers in its body.
  Use the protected merge path and satisfy the live branch rules. A cancelled,
  skipped, or timed-out check is not a successful release validation.
- [ ] Confirm that every configured Linux, macOS, and Windows runner label can
  execute its release job. Dispatch both artifact workflows on the candidate
  branch, and confirm each completed run reports the intended candidate SHA.
  Each packaged CLI must execute the committed UART firmware, and each DAP must
  complete an initialize/disconnect exchange. Python wheels must boot the same
  firmware on their native architecture, including Linux ARM64:

  ```bash
  gh workflow run core-release.yml --repo w1ne/labwired-core --ref release/vX.Y.Z
  gh workflow run core-python-release.yml --repo w1ne/labwired-core --ref release/vX.Y.Z
  ```

  These dispatches build and check artifacts. They do not publish to GitHub
  Releases, GHCR, or PyPI, including when a manual dispatch uses a tag ref.
- [ ] Create or update a GitHub Release **draft before pushing the release tag**.
  Use the title and body from the tracked notes file. Keep the draft associated
  with the intended candidate and record any validation still pending.

## 3. Merge and publish

- [ ] Merge the reviewed release PR through branch protection. Record the merged
  commit SHA and validate that exact commit, including the CLI and Python
  artifact dry-runs. If the candidate changes, repeat the affected checks and
  update the release draft.
- [ ] Confirm the merged commit is on `main`, its workspace version is `X.Y.Z`,
  and its tracked notes are the reviewed release notes.
- [ ] When publication is authorized, create the annotated tag at the verified
  commit and push it. Replace both placeholders below before running:

  ```bash
  git tag -a vX.Y.Z VERIFIED_MERGED_SHA -m "Release vX.Y.Z"
  git push origin refs/tags/vX.Y.Z
  ```

- [ ] Monitor both tag-triggered workflows:
  [core-release.yml](.github/workflows/core-release.yml) and
  [core-python-release.yml](.github/workflows/core-python-release.yml).
  The CLI workflow rejects a tag/version mismatch and missing tracked notes.
  It takes the title and body from `docs/releases/vX.Y.Z.md`, uploads the
  validated archives, and publishes the prepared GitHub Release draft. The
  Python workflow checks the artifact versions against the tag before
  publishing through the `pypi` Trusted Publishing environment.

## 4. Verify the published artifacts

- [ ] Confirm the CLI workflow attached all five
  `labwired-vX.Y.Z-<platform>.tar.gz` archives:

  | Platform | Required archive contents |
  | :--- | :--- |
  | Linux x86_64 | `labwired`, `labwired-dap` |
  | Linux aarch64 | `labwired`, `labwired-dap` |
  | macOS x86_64 | `labwired`, `labwired-dap` |
  | macOS aarch64 | `labwired`, `labwired-dap` |
  | Windows x86_64 | `labwired.exe`, `labwired-dap.exe` |

- [ ] Confirm the Linux glibc floor checks and all five older-distribution
  smoke cases passed. On every macOS, Windows and Linux target, verify that the
  packaged CLI reports the intended version and executes the UART firmware,
  and that the DAP completes its initialize/disconnect protocol exchange.
- [ ] Confirm GHCR contains `ghcr.io/w1ne/labwired:vX.Y.Z` for Linux amd64 and
  arm64. Use the immutable version tag in CI. The workflow also updates its
  moving convenience tag.
- [ ] Confirm the anonymous GHCR pull and released runner/action smoke tests
  passed. If a first publication fails because the package is private, set the
  package visibility to public and rerun the failed checks for that same tag.
- [ ] Confirm [PyPI](https://pypi.org/project/labwired/) contains `X.Y.Z` with
  five `cp39-abi3` wheels and one sdist: Linux x86_64/aarch64, macOS arm64/x86_64,
  and Windows x64.
- [ ] Test the public installation path and the explicitly versioned install.
  The presence of a GitHub Release does not prove that GHCR or PyPI publication
  completed successfully.

For a failed workflow, diagnose and repair the failed stage. Infrastructure
failures can be retried on the same verified tag. A source defect requires a new
reviewed candidate; do not move a published tag or replace the release with
manually packaged, unverified binaries.

### Historical runner image backfill

The `v0.18.0` tag predates automatic runner image publication. If that historical
image needs its initial backfill, explicitly dispatch
[core-backfill-runner-image.yml](.github/workflows/core-backfill-runner-image.yml)
with `version` set to `v0.18.0`. The workflow builds the exact tagged source,
publishes only `ghcr.io/w1ne/labwired:v0.18.0`, and verifies an anonymous
pull-and-run. It never updates `latest`. New core release tags use the normal
release workflow instead.

## 5. Downstream updates and announcement

- [ ] Record the release URLs, final source SHA, and completed artifact checks.
- [ ] Update and verify the web application, VS Code extension, or firmware demo
  bundles in their own repositories when those updates are in scope. A core
  release does not deploy those products automatically.
- [ ] Announce the verified release through the authorized channels.
