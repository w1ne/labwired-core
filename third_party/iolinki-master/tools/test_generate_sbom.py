#!/usr/bin/env python3
"""Tests for generate_sbom.py — per-release CycloneDX/SPDX SBOM generator."""

import json
import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from generate_sbom import LICENSE_EXPRESSION, build_cyclonedx, build_spdx  # noqa: E402

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "generate_sbom.py")
VERSION = "0.2.0"


class TestCycloneDX(unittest.TestCase):
    def setUp(self):
        self.doc = build_cyclonedx(VERSION)

    def test_format_and_spec_version(self):
        self.assertEqual(self.doc["bomFormat"], "CycloneDX")
        self.assertEqual(self.doc["specVersion"], "1.6")

    def test_root_component_identity(self):
        comp = self.doc["metadata"]["component"]
        self.assertEqual(comp["name"], "iolinki-master")
        self.assertEqual(comp["version"], VERSION)
        self.assertEqual(comp["purl"], f"pkg:github/w1ne/iolinki-master@v{VERSION}")
        self.assertEqual(comp["licenses"][0]["expression"], LICENSE_EXPRESSION)

    def test_zero_runtime_dependencies_is_explicit(self):
        root_ref = self.doc["metadata"]["component"]["bom-ref"]
        deps = {d["ref"]: d.get("dependsOn", []) for d in self.doc["dependencies"]}
        self.assertIn(root_ref, deps)
        self.assertEqual(deps[root_ref], [])

    def test_build_tools_are_excluded_scope(self):
        names = {c["name"]: c for c in self.doc.get("components", [])}
        for tool in ("cmake", "cmocka", "iolinki"):
            self.assertIn(tool, names)
            self.assertEqual(names[tool]["scope"], "excluded")


class TestSPDX(unittest.TestCase):
    def setUp(self):
        self.doc = build_spdx(VERSION)

    def test_document_identity(self):
        self.assertEqual(self.doc["spdxVersion"], "SPDX-2.3")
        self.assertEqual(self.doc["SPDXID"], "SPDXRef-DOCUMENT")
        self.assertEqual(self.doc["dataLicense"], "CC0-1.0")

    def test_root_package(self):
        pkgs = {p["name"]: p for p in self.doc["packages"]}
        self.assertIn("iolinki-master", pkgs)
        pkg = pkgs["iolinki-master"]
        self.assertEqual(pkg["versionInfo"], VERSION)
        self.assertEqual(pkg["licenseDeclared"], LICENSE_EXPRESSION)
        purls = [
            r["referenceLocator"]
            for r in pkg.get("externalRefs", [])
            if r["referenceType"] == "purl"
        ]
        self.assertEqual(purls, [f"pkg:github/w1ne/iolinki-master@v{VERSION}"])

    def test_describes_relationship(self):
        rels = [
            r
            for r in self.doc["relationships"]
            if r["relationshipType"] == "DESCRIBES"
            and r["spdxElementId"] == "SPDXRef-DOCUMENT"
        ]
        self.assertEqual(len(rels), 1)


class TestCLI(unittest.TestCase):
    def run_tool(self, *args):
        return subprocess.run(
            [sys.executable, TOOL, *args], capture_output=True, text=True
        )

    def test_writes_valid_json_for_both_formats(self):
        for fmt in ("cyclonedx", "spdx"):
            with tempfile.TemporaryDirectory() as tmp:
                out = os.path.join(tmp, "sbom.json")
                res = self.run_tool(
                    "--version", VERSION, "--format", fmt, "--output", out
                )
                self.assertEqual(res.returncode, 0, res.stderr)
                with open(out, encoding="utf-8") as fh:
                    json.load(fh)

    def test_unknown_format_fails(self):
        res = self.run_tool("--version", VERSION, "--format", "xml", "--output", "x")
        self.assertNotEqual(res.returncode, 0)


if __name__ == "__main__":
    unittest.main()
