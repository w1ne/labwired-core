#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Pure policy and tiny Git-history controls; no compiler, profiler or guest."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import pr_hotpath_gate as gate


class PolicyControls(unittest.TestCase):
    def test_runtime_build_descriptor_and_fixture_changes_select(self):
        paths = ["crates/core/src/bus/samd51_storage.rs", "crates/core/src/cpu/avr.rs",
                 "crates/core/src/machine/boundary.rs", "crates/core/src/lib.rs",
                 "crates/core/src/peripherals/sam/spi.rs", "crates/cli/src/main.rs",
                 "crates/config/src/lib.rs", "configs/chips/atmega328p.yaml",
                 "configs/systems/pybadge.yaml",
                 "Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml",
                 "crates/core/Cargo.toml", "crates/cli/build.rs",
                 ".github/workflows/core-perf.yml", "scripts/perf/board_perf.py"]
        paths += [p + "main.c" for p in gate.FIRMWARE_PREFIXES]
        for path in paths:
            with self.subTest(path=path):
                self.assertTrue(gate.select("pull_request", gate.OWNED_REPOSITORY, "fix/x", [path])[0])

    def test_docs_and_separately_compiled_tests_skip(self):
        paths = ["README.md", "docs/engineering/target-next-lanes.md", "crates/core/tests/example.rs",
                 "crates/core/src/tests/scb_reset.rs", "scripts/unrelated.py"]
        self.assertFalse(gate.select("pull_request", gate.OWNED_REPOSITORY, "docs/x", paths)[0])

    def test_existing_event_and_upstream_sync_admission(self):
        for event in ["push", "schedule", "workflow_dispatch"]:
            self.assertTrue(gate.select(event, gate.OWNED_REPOSITORY, "", [])[0])
        self.assertTrue(gate.select("pull_request", gate.OWNED_REPOSITORY, "merge/upstream-x", [])[0])
        self.assertTrue(gate.select("pull_request", gate.OWNED_REPOSITORY, "up/crisp-sync", [])[0])
        self.assertFalse(
            gate.select(
                "pull_request", "CrispStrobe/labwired-core", "fix/x", ["crates/core/src/lib.rs"]
            )[0]
        )


class HistoryControls(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name)
        self.old = Path.cwd()
        os.chdir(self.repo)
        self.addCleanup(os.chdir, self.old)
        self.run_git("init", "-b", "main")
        self.write("crates/core/src/lib.rs", "runtime\n")
        self.run_git("add", ".")
        self.run_git("commit", "-m", "base")
        self.initial = self.sha()

    def run_git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "gc.auto=0", "-c", "maintenance.auto=false",
             "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
             "-c", "user.name=Guard Test", "-c", "user.email=guard@example.invalid",
             *args], stderr=subprocess.PIPE).decode().strip()

    def write(self, name, value):
        p = self.repo / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(value)

    def sha(self):
        return self.run_git("rev-parse", "HEAD")

    def test_docs_branch_behind_main_does_not_include_foreign_runtime_change(self):
        self.run_git("checkout", "-b", "docs")
        self.write("README.md", "documentation\n")
        self.run_git("add", ".")
        self.run_git("commit", "-m", "docs")
        head = self.sha()
        self.run_git("checkout", "main")
        self.write("crates/core/src/lib.rs", "new main runtime\n")
        self.run_git("commit", "-am", "main runtime")
        base = self.sha()
        self.run_git("merge", "--no-ff", "docs", "-m", "tested merge")
        paths = gate.changed_paths(self.sha(), base, head)
        self.assertEqual(paths, ["README.md"])
        self.assertFalse(any(map(gate.hotpath, paths)))

    def test_runtime_rename_out_of_scope_still_selects_and_wrong_parents_refuse(self):
        self.run_git("checkout", "-b", "move")
        self.run_git("mv", "crates/core/src/lib.rs", "README.md")
        self.run_git("commit", "-m", "rename")
        head = self.sha()
        self.run_git("checkout", "main")
        self.run_git("merge", "--no-ff", "move", "-m", "tested merge")
        tested = self.sha()
        self.assertTrue(any(map(gate.hotpath, gate.changed_paths(tested, self.initial, head))))
        for args in [(tested, head, self.initial), (self.initial, self.initial, head), (tested, "bad", head)]:
            with self.subTest(args=args), self.assertRaises(ValueError):
                gate.changed_paths(*args)

    def make_refreshed_merge(self, runtime_change=False):
        self.run_git("checkout", "-b", "feature")
        name = "crates/core/src/new_route.rs" if runtime_change else "README.md"
        self.write(name, "feature change\n")
        self.run_git("add", ".")
        self.run_git("commit", "-m", "feature")
        head = self.sha()
        self.run_git("checkout", "main")
        self.write("crates/core/src/lib.rs", "new main runtime\n")
        self.run_git("commit", "-am", "main advanced after event")
        actual_base = self.sha()
        self.run_git("merge", "--no-ff", "feature", "-m", "refreshed tested merge")
        return self.sha(), actual_base, head, name

    def test_refreshed_base_uses_actual_first_parent_not_stale_event_base(self):
        tested, _, head, name = self.make_refreshed_merge()
        paths = gate.changed_paths(tested, self.initial, head)
        self.assertEqual(paths, [name])
        self.assertFalse(any(map(gate.hotpath, paths)))

    def test_refreshed_base_still_requires_runtime_performance(self):
        tested, _, head, name = self.make_refreshed_merge(runtime_change=True)
        paths = gate.changed_paths(tested, self.initial, head)
        self.assertEqual(paths, [name])
        self.assertTrue(gate.select("pull_request", gate.OWNED_REPOSITORY, "fix/x", paths)[0])

    def test_refreshed_base_rejects_wrong_head_and_unrelated_base(self):
        tested, actual_base, head, _ = self.make_refreshed_merge()
        with self.assertRaises(ValueError):
            gate.changed_paths(tested, self.initial, actual_base)
        self.run_git("checkout", "-b", "unrelated", self.initial)
        self.write("other.txt", "unrelated history\n")
        self.run_git("add", ".")
        self.run_git("commit", "-m", "different branch, not an ancestor")
        unrelated = self.sha()
        self.run_git("checkout", tested)
        with self.assertRaises(subprocess.CalledProcessError):
            gate.changed_paths(tested, unrelated, head)

    def test_refreshed_base_requires_complete_ancestry_proof(self):
        tested, _, head, _ = self.make_refreshed_merge()
        real_git = gate.git
        def missing_ancestry(*args):
            if args[:2] == ("merge-base", "--is-ancestor"):
                raise subprocess.CalledProcessError(128, "git", stderr=b"missing history")
            return real_git(*args)
        with patch.object(gate, "git", side_effect=missing_ancestry):
            with self.assertRaises(subprocess.CalledProcessError):
                gate.changed_paths(tested, self.initial, head)

    def test_git_failure_is_not_a_false_skip(self):
        with patch.object(gate, "git", side_effect=subprocess.CalledProcessError(1, "git")):
            with self.assertRaises(subprocess.CalledProcessError):
                gate.changed_paths(self.initial, self.initial, self.initial)


if __name__ == "__main__":
    unittest.main(verbosity=2)
