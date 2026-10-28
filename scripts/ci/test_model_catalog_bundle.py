#!/usr/bin/env python3
"""Tests for the public model catalog bundle builder."""

import importlib.util
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
import tarfile


BUILDER_PATH = Path(__file__).with_name("build_model_catalog_bundle.py")
WORKFLOW_PATH = Path(__file__).resolve().parents[2] / ".github/workflows/publish-model-catalog.yml"
SPEC = (
    importlib.util.spec_from_file_location("build_model_catalog_bundle", BUILDER_PATH)
    if BUILDER_PATH.is_file()
    else None
)
BUILDER = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC and SPEC.loader and BUILDER:
    SPEC.loader.exec_module(BUILDER)


class ModelCatalogBundleTest(unittest.TestCase):
    def test_bundle_contains_only_configs_and_manifest_matches_archive(self):
        self.assertIsNotNone(BUILDER, "bundle builder module must be available")
        self.assertTrue(hasattr(BUILDER, "build_bundle"), "build_bundle must be implemented")

        with tempfile.TemporaryDirectory() as temporary_dir:
            root = Path(temporary_dir)
            configs = root / "configs"
            (configs / "chips").mkdir(parents=True)
            (configs / "systems").mkdir(parents=True)
            (configs / "chips" / "alpha.yaml").write_text("chip: alpha\n")
            (configs / "systems" / "alpha.yaml").write_text("chip: alpha\n")
            (root / "README.md").write_text("unrelated repository file\n")
            output = root / "bundle"

            BUILDER.build_bundle(
                configs_dir=configs,
                chips=["zeta", "alpha"],
                core_commit="0123456789abcdef0123456789abcdef01234567",
                output_dir=output,
            )

            archive_path = output / "configs.tar.gz"
            manifest = json.loads((output / "catalog.json").read_text())
            with tarfile.open(archive_path, "r:gz") as archive:
                names = archive.getnames()

            self.assertTrue(all(name == "configs" or name.startswith("configs/") for name in names))
            self.assertIn("configs/chips/alpha.yaml", names)
            self.assertIn("configs/systems/alpha.yaml", names)
            self.assertNotIn("README.md", names)
            self.assertEqual(
                manifest,
                {
                    "schema_version": 1,
                    "core_commit": "0123456789abcdef0123456789abcdef01234567",
                    "archive": "configs.tar.gz",
                    "sha256": hashlib.sha256(archive_path.read_bytes()).hexdigest(),
                    "chips": ["alpha", "zeta"],
                },
            )


class ModelCatalogWorkflowTest(unittest.TestCase):
    def test_manual_dispatch_is_removed_and_checkout_uses_main(self):
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertNotIn("workflow_dispatch:", workflow)
        checkout_step = workflow.split("- uses: actions/checkout@v4", 1)[1].split("\n\n", 1)[0]
        self.assertIn("ref: refs/heads/main", checkout_step)
        self.assertIn("github.ref != 'refs/heads/main'", workflow)
        self.assertLess(
            workflow.index("- name: Require main ref"),
            workflow.index("- uses: actions/checkout@v4"),
        )
        self.assertIn("exit 1", workflow)

    def test_bundle_commit_and_publication_message_use_checked_out_head(self):
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn('echo "CORE_COMMIT=$(git rev-parse HEAD)" >> "$GITHUB_ENV"', workflow)
        self.assertIn('--core-commit "$CORE_COMMIT"', workflow)
        self.assertIn('git commit -m "Publish model catalog for $CORE_COMMIT"', workflow)

    def test_push_triggers_include_catalog_discovery_sources(self):
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        push_paths = workflow.split("    paths:\n", 1)[1].split("\n\npermissions:", 1)[0]
        for path in (
            "crates/core/src/session/catalog.rs",
            "crates/core/examples/export_catalog_names.rs",
        ):
            with self.subTest(path=path):
                self.assertIn(f'      - "{path}"', push_paths)


if __name__ == "__main__":
    unittest.main()
