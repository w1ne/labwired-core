# Public Model Catalog Publisher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Publish only the public `configs/` tree plus chip names from the existing core `Catalog` as a versioned download for the VS Code extension.

**Architecture:** A tiny Rust example prints `Catalog::chip_names()` so the workflow uses the existing discovery implementation. A tested Python builder creates `configs.tar.gz` and `catalog.json` with the core commit, chip names, archive name, and SHA-256. A workflow updates a dedicated `model-catalog` branch whenever public configs change.

**Tech Stack:** Rust, Python 3 standard library, GitHub Actions, tar.gz.

---

## File map

- `crates/core/examples/export_catalog_names.rs`: emit the actual core catalog names as JSON.
- `scripts/ci/build_model_catalog_bundle.py`: write the public configs archive and metadata.
- `scripts/ci/test_model_catalog_bundle.py`: test archive contents, metadata, and checksum.
- `.github/workflows/publish-model-catalog.yml`: test, build, and publish the bundle.

## Task 1: Export names from the existing Catalog

**Files:** Create `crates/core/examples/export_catalog_names.rs`.

- [ ] Print `labwired_core::session::catalog::Catalog::discover().chip_names()` as one JSON array on stdout using `serde_json`.
- [ ] Run `cargo run -p labwired-core --example export_catalog_names`; expect a sorted JSON array that includes `stm32f401` and matches `configs/chips/*.yaml` stems.

## Task 2: Build and test the public bundle

**Files:** Create `scripts/ci/build_model_catalog_bundle.py` and `scripts/ci/test_model_catalog_bundle.py`.

- [ ] Write a failing standard-library `unittest` using temporary `configs/chips/alpha.yaml`, `configs/systems/alpha.yaml`, and an unrelated `README.md`. Assert the archive contains only the `configs/` tree, the manifest includes the passed `core_commit` and sorted `chips`, and `sha256` matches the archive bytes.
- [ ] Run `python3 scripts/ci/test_model_catalog_bundle.py`; expect failure because the builder does not exist.
- [ ] Implement `build_bundle(configs_dir: Path, chips: list[str], core_commit: str, output_dir: Path)`. Write `configs.tar.gz` and `catalog.json` with this shape:

```json
{
  "schema_version": 1,
  "core_commit": "<40-character commit SHA>",
  "archive": "configs.tar.gz",
  "sha256": "<64-character lowercase hex SHA-256>",
  "chips": ["alpha"]
}
```

- [ ] Run `python3 scripts/ci/test_model_catalog_bundle.py`; expect all tests to pass.

## Task 3: Publish on public catalog changes

**Files:** Create `.github/workflows/publish-model-catalog.yml`.

- [ ] Trigger on pushes to `main` that change `configs/**`, the bundle builder, or this workflow; also support `workflow_dispatch` for the first publication.
- [ ] Grant only `contents: write` and serialize publications with a workflow concurrency group.
- [ ] Run the bundle unit test, then run `cargo run --quiet -p labwired-core --example export_catalog_names` and pass its JSON to the bundle builder with `${GITHUB_SHA}`.
- [ ] Publish only `catalog.json` and `configs.tar.gz` at the root of the `model-catalog` branch. Replace that branch's contents on each publication so it contains no core source files.
- [ ] Run `actionlint .github/workflows/publish-model-catalog.yml` when available and inspect the workflow diff for branch and permission scope.

## Task 4: Verify the publisher end to end

**Files:** Verify the files above.

- [ ] Run `python3 scripts/ci/test_model_catalog_bundle.py`.
- [ ] Run `cargo run --quiet -p labwired-core --example export_catalog_names` and validate the output with `python3 -m json.tool`.
- [ ] Run the workflow's archive and manifest commands locally with the current commit; verify every manifest chip has `configs/chips/<chip>.yaml` in the archive and no repository paths outside `configs/` are present.
- [ ] Commit as `feat(core): publish model catalog bundle`.
