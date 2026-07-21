# iolinki-master Release Strategy

The master stack uses a **single-branch** model — simpler than the device stack's
Gitflow. There is no `develop`. All release-ready work lives on `master`; features
land through short-lived branches and PRs (see [`CONTRIBUTING.md`](CONTRIBUTING.md)).
A release is a tag on `master`.

## 1. Branches

- **`master`** — the one long-lived branch. Stable, releasable, protected. Version
  tags (`vX.Y.Z`) are pushed from here.
- **feature / fix branches** — short-lived, created from `master`, merged back via
  PR after CI is green.

## 2. Versioning

**Semantic Versioning 2.0.0**, `MAJOR.MINOR.PATCH`:

- **MAJOR** — incompatible public-API changes (the `include/iolinki_master/master.h`
  contract, opaque storage sizes, or result-code meanings).
- **MINOR** — backward-compatible new functionality (new services, new diagnostics).
- **PATCH** — backward-compatible bug fixes.

Pre-1.0, the API is still moving; minor versions may tighten contracts as the
timing/scheduler layer stabilizes. Keep the project version in `CMakeLists.txt`
(`project(... VERSION ...)`) in step with the tag.

## 3. What gates a release

A tag is only cut when, on the exact `master` SHA being tagged:

- The full local gate is green: `cmake -S . -B build && cmake --build build &&
  ctest --test-dir build --output-on-failure && git diff --check`.
- Both CI jobs are green on that SHA: `cmake-ctest` and
  `labwired-real-firmware-model`.
- `CHANGELOG.md` has the release's entry moved out of `[Unreleased]` into a dated
  `vX.Y.Z` section, and `IMPLEMENTATION_STATUS.md` reflects reality — no feature is
  claimed as more mature than its evidence. In particular, do **not** claim
  hardware, timing, or IO-Link-conformance validation that has not happened.

## 4. Release process

```bash
git checkout master
git pull origin master
# bump project() VERSION in CMakeLists.txt, finalize CHANGELOG.md, commit via PR
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

Pushing a `v*` tag triggers `release.yml`, which:

- builds and tests the stack with **code coverage**,
- generates **SBOMs** in both **CycloneDX and SPDX** (shipping from `0.2`),
- generates **release notes** from the conventional-commit history,
- creates the **GitHub Release** with the notes, SBOMs, and coverage attached.

The SBOMs record the two source origins — this repository and the pinned `iolinki`
frame/CRC helper sources — making explicit that the stack has zero third-party
runtime dependencies (see [`security/CRA.md`](security/CRA.md)).

### Failed release

If CI fails on a tag, delete the tag locally and on the remote, fix on `master`, and
re-tag:

```bash
git tag -d v0.2.0
git push origin :v0.2.0
# fix on master, then re-tag
```

## 5. Release artifacts

Each GitHub Release includes: the auto-generated source archive, the CycloneDX +
SPDX SBOMs, the coverage summary, and the generated release notes. Security fixes
ship as tagged releases with a `CHANGELOG.md` entry and, for confirmed
vulnerabilities, a GitHub security advisory (see [`SECURITY.md`](../SECURITY.md)).

## 6. Security and CRA

Supported versions and the coordinated-disclosure process are defined in
[`SECURITY.md`](../SECURITY.md); the CRA division of labor and the SBOM commitment
are in [`security/CRA.md`](security/CRA.md). The per-release SBOMs are the
machine-readable half of that commitment and begin with the `0.2` release.
