# LabWired Core Release and Merge Strategy

Release preparation produces a reviewed version change, changelog, and tracked
release notes. Publication starts only after the release PR has merged and the
candidate commit has passed validation. The detailed artifact checklist is in
[RELEASE_PROCESS.md](../RELEASE_PROCESS.md).

## Branches and merge rules

- Develop on short-lived branches from `main`. Use a separate worktree when an
  existing checkout contains another task's changes.
- Prepare a release on `release/vX.Y.Z`. Merge changes through a pull request.
  Do not push release preparation commits directly to `main`.
- Follow the repository's current branch protection. On 2026-11-10, `main`
  requires `pr-gate`, `release-runner-contract`, `browser-layer`, and
  `pr-core-feature-off`, with an up-to-date base and administrator enforcement.
  Check the live rules before merging; this list is a dated observation.
- Review all candidate checks, including the workspace test shards and
  `core-integrity`. A cancelled or timed-out run does not establish a pass.
- Resolve release blockers or document an explicit release scope decision. Do
  not remove checks, weaken assertions, or add waivers to obtain a green run.

## Prepare the candidate

1. Record the current stable core tag and candidate commit SHA. Core release tags
   use `vMAJOR.MINOR.PATCH`; `firmware-demos-*` tags are separate artifacts.
2. Review the changes since the stable tag. Include only merged changes and
   describe compatibility changes and known limits. A descriptor or catalog
   entry alone does not prove that firmware or every peripheral works.
3. Choose the next version. Use a patch for compatible fixes and a minor version
   for new capabilities. While the project is at `0.x`, state API and behavior
   changes explicitly; do not claim compatibility without evidence.
4. Update `[workspace.package].version` in `Cargo.toml`, the applicable internal
   package references, and `Cargo.lock`. Preserve `version.workspace = true` in
   member manifests. Confirm the inherited versions with
   `cargo metadata --locked --no-deps --format-version 1`.
5. Add the dated release section to `CHANGELOG.md` and leave a new `Unreleased`
   section for later work. Keep installation references and release contract
   expectations consistent with the prepared version. Preserve historical
   immutable action pins unless they are deliberately replaced and verified.
6. Add `docs/releases/vX.Y.Z.md` to Git. This is the source for the GitHub Release
   title and body. Start the file with `# Release vX.Y.Z: <short summary>` and use
   the following sections:

   ```markdown
   # Release vX.Y.Z: <short summary>

   Features:
   - <Feature and its scope>

   Improvements:
   - <Improvement and its effect>

   Fixes:
   - <Fixed behavior>
   ```

   Omit `Fixes:` when no fixes apply. Add compatibility, known-limit, and
   installation information when needed. Link to the tagged changelog and
   compare range with absolute GitHub URLs.
7. Commit and push the candidate branch, then open a release preparation PR.
   Record the validation results and remaining blockers in its description.

## Validate the candidate

Use the pinned toolchain in `rust-toolchain.toml`. Follow
[core-ci.yml](../.github/workflows/core-ci.yml) for the required target libraries,
C cross-compiler, device-tree tools, and firmware fixtures. The workspace has
many firmware packages; excluding only a package named `firmware` is not a
complete host-test configuration.

Start with the release and installer contracts, formatting, metadata, and a
locked CLI/DAP build:

```bash
bash scripts/ci/verify-release-runner-contract.sh
bash scripts/ci/test-install-script.sh
cargo fmt --all -- --check
cargo metadata --locked --no-deps --format-version 1
cargo build -p labwired-cli -p labwired-dap --release --locked
```

Run the PR test shards and their aggregate with the same prerequisites as CI.
The shard script and `scripts/ci/workspace-test-shards.json` define the test
allocation, firmware-only exclusions, and issue-linked known failures. Keep the
reports so that passed, skipped, and known-failing tests remain distinguishable.
Also run the full integrity/feature checks and the firmware-backed full suite
from `core-ci.yml`. Record lint, board-coverage, performance, and documentation
results. Do not replace these checks with a CLI startup test.

Before tagging, dispatch both release workflows against the candidate branch:

```bash
gh workflow run core-release.yml --repo w1ne/labwired-core --ref release/vX.Y.Z
gh workflow run core-python-release.yml --repo w1ne/labwired-core --ref release/vX.Y.Z
```

Wait for both runs to finish and verify their `headSha` matches the intended
candidate. A new commit requires a new candidate check. `core-release.yml`
validates the workspace version and tracked notes, builds five CLI/DAP archives,
checks the Linux glibc floor, and starts the Linux binaries on older distributions.
`core-python-release.yml` builds five Python wheels plus an sdist and runs its
artifact checks. A manual dispatch does not publish either workflow's artifacts
to GitHub Releases, GHCR, or PyPI.

The release matrices need Linux x86_64 and ARM64, macOS, and Windows execution.
Confirm that every configured runner label can start a job before the release.
Use available self-hosted infrastructure where it satisfies the target and
container requirements; do not substitute a Linux runner for macOS or Windows.
Runner configuration is a separate prerequisite, not a reason to drop a platform
or skip a build.

## Merge, tag, and publish

1. Merge the release PR through the protected path after review and successful
   candidate validation. Record the merged commit SHA.
2. Validate that exact merged commit. Confirm its workspace version, changelog,
   notes, tests, and artifact builds match the intended release. Prepare or
   update a GitHub Release draft with the tracked notes for review.
3. Create the annotated `vX.Y.Z` tag at the verified commit on `main`, then push
   it when publication is authorized. Do not tag a moving branch without first
   checking the commit SHA.
4. A tag push runs `core-release.yml` and `core-python-release.yml`. The first
   publishes five CLI/DAP archives and the Linux amd64/arm64 CI runner image.
   It reads the GitHub Release title and body from `docs/releases/vX.Y.Z.md`.
   The second publishes the Python SDK through the `pypi` Trusted Publishing
   environment. Each workflow must complete successfully.
5. Verify all five archives contain both `labwired` and `labwired-dap` (with
   `.exe` on Windows). Confirm the release version, Linux glibc checks, anonymous
   pull of `ghcr.io/w1ne/labwired:vX.Y.Z`, and release action smoke tests. Confirm
   PyPI contains the matching version, five wheels, and one sdist.
6. Check the public installation path against the published release. A GitHub
   Release can exist before the runner image or PyPI publication finishes; its
   existence alone is not proof of a complete release.

A core release does not by itself deploy the LabWired web application, VS Code
extension, or firmware demo bundles. Prepare and verify those downstream updates
in their own repositories and workflows when they are in scope.

## Coding standards

Use Conventional Commits, standard Rust formatting, and documented public APIs.
Use `thiserror` for library errors and `anyhow` for application/CLI error context.
Describe measured results and current limitations in release notes.
