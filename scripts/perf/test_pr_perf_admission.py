#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise real admission scripts and workflow wiring without compiling Rust."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

import pr_perf_verdict as verdict
from test_pr_hotpath_gate import HistoryControls

ROOT = Path(__file__).resolve().parents[2]


def workflow(name):
    return yaml.safe_load((ROOT / ".github/workflows" / name).read_text())


class VerdictControls(unittest.TestCase):
    def test_measurement_must_succeed_when_required(self):
        self.assertEqual(verdict.check("success", "true", "success", "false"), "measured")
        for result in ["failure", "skipped", "cancelled", "timed_out", "", "neutral"]:
            with self.subTest(result=result), self.assertRaises(ValueError):
                verdict.check("success", "true", result, "false")

    def test_no_measurement_is_only_an_explicit_scoped_skip(self):
        self.assertEqual(verdict.check("success", "false", "skipped", "false"), "not-required")
        for result in ["success", "failure", "cancelled", ""]:
            with self.subTest(result=result), self.assertRaises(ValueError):
                verdict.check("success", "false", result, "false")

    def test_failed_scope_and_missing_or_invalid_outputs_refuse(self):
        for scope in ["failure", "cancelled", "skipped", ""]:
            with self.subTest(scope=scope), self.assertRaises(ValueError):
                verdict.check(scope, "", "skipped", "false")
        for required in ["", "False", "0", " true", "true\nfalse"]:
            with self.subTest(required=required), self.assertRaises(ValueError):
                verdict.check("success", required, "skipped", "false")
        with self.assertRaises(ValueError):
            verdict.check("success", "false", "skipped", "")

    def test_drafts_only_accept_a_scoped_skip(self):
        self.assertEqual(verdict.check("success", "true", "skipped", "true"), "draft-skipped")
        with self.assertRaises(ValueError):
            verdict.check("success", "true", "success", "true")


class WorkflowControls(unittest.TestCase):
    def test_required_aggregate_uses_real_parallel_perf_workflow(self):
        ci = workflow("core-ci.yml")
        perf = ci["jobs"]["pr-performance"]
        self.assertEqual(perf["uses"], "./.github/workflows/core-perf-pr.yml")
        aggregate = ci["jobs"]["pr-core-feature-off"]
        self.assertEqual(aggregate["needs"], [
            "pr-default-members-lib", "pr-core-feature-off-tests",
            "pr-ngspice-differential", "pr-performance",
        ])
        self.assertIn("always()", aggregate["if"])
        self.assertIn("draft-light", aggregate["name"])
        self.assertIn("ready_for_review", ci.get("on", ci.get(True))["pull_request"]["types"])
        step = aggregate["steps"][0]
        self.assertEqual(step["env"]["PERF_RESULT"], "${{ needs.pr-performance.result }}")
        self.assertEqual(step["env"]["NGSPICE_RESULT"], "${{ needs.pr-ngspice-differential.result }}")
        ngspice = ci["jobs"]["pr-ngspice-differential"]
        self.assertFalse(ngspice.get("continue-on-error", False))
        self.assertEqual(
            " ".join(ngspice["if"].split()),
            "!(github.event_name == 'pull_request' && github.event.pull_request.draft) && "
            "(github.event_name == 'pull_request' || github.event_name == 'workflow_dispatch')",
        )
        self.assertTrue(any(
            "apt-get install -y --no-install-recommends ngspice libngspice0" in item.get("run", "")
            for item in ngspice["steps"]
        ))
        self.assertTrue(any(
            item.get("env", {}).get("LABWIRED_REQUIRE_NGSPICE") == "1"
            and "--test analog_vs_ngspice_differential" in item.get("run", "")
            for item in ngspice["steps"]
        ))
        worker_results = {
            "DEFAULT_MEMBERS_RESULT": "success",
            "FEATURE_OFF_RESULT": "success",
            "NGSPICE_RESULT": "success",
            "PERF_RESULT": "success",
        }
        worker_jobs = {
            "DEFAULT_MEMBERS_RESULT": "pr-default-members-lib",
            "FEATURE_OFF_RESULT": "pr-core-feature-off-tests",
            "NGSPICE_RESULT": "pr-ngspice-differential",
            "PERF_RESULT": "pr-performance",
        }
        for variable, job in worker_jobs.items():
            self.assertEqual(step["env"][variable], f"${{{{ needs.{job}.result }}}}")
            for result in ["failure", "skipped", "cancelled", ""]:
                env = dict(os.environ, **worker_results)
                env[variable] = result
                proc = subprocess.run(["bash", "-e", "-c", step["run"]], env=env, capture_output=True, text=True)
                self.assertNotEqual(proc.returncode, 0, f"{variable}={result!r} was admitted")
        proc = subprocess.run(
            ["bash", "-e", "-c", step["run"]],
            env=dict(os.environ, **worker_results), capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)

    def test_admission_verdict_runs_after_failures_and_has_a_distinct_draft_name(self):
        wf = workflow("core-perf-pr.yml")
        self.assertIn("workflow_call", wf.get("on", wf.get(True)))
        jobs = wf["jobs"]
        final = jobs["perf-verdict"]
        self.assertEqual(final["if"], "${{ always() }}")
        self.assertEqual(final["needs"], ["perf-scope", "pr-throughput"])
        self.assertIn("draft-skipped", final["name"])
        step = final["steps"][-1]
        self.assertIn("pr_perf_verdict.py", step["run"])
        self.assertEqual(step["env"]["REQUIRED"], "${{ needs.perf-scope.outputs.required }}")
        self.assertEqual(step["env"]["MEASUREMENT_RESULT"], "${{ needs.pr-throughput.result }}")
        self.assertEqual(jobs["pr-throughput"]["if"], "${{ needs.perf-scope.outputs.required == 'true' && !(github.event_name == 'pull_request' && github.event.pull_request.draft) }}")

    def test_python_is_pinned_before_dependency_installation(self):
        jobs = workflow("core-perf-pr.yml")["jobs"]
        for name in ["perf-scope", "pr-throughput"]:
            with self.subTest(job=name):
                steps = jobs[name]["steps"]
                setup = [i for i, step in enumerate(steps)
                         if step.get("uses") == "actions/setup-python@v6"]
                self.assertEqual(len(setup), 1)
                self.assertEqual(steps[setup[0]]["with"]["python-version"], "3.12")
                pip = [i for i, step in enumerate(steps)
                       if "pip install" in step.get("run", "")]
                self.assertTrue(pip)
                self.assertTrue(all(setup[0] < i for i in pip))

    def test_short_measurement_keeps_existing_thresholds_and_all_modes(self):
        job = workflow("core-perf-pr.yml")["jobs"]["pr-throughput"]
        commands = "\n".join(step.get("run", "") for step in job["steps"])
        self.assertEqual(job["runs-on"], "ubuntu-24.04")
        self.assertEqual(job["env"]["PERF_BOARDS"], "esp32c3,rp2040,atmega328p")
        self.assertIn("--repeats 3 --seconds 1 --min-rtx 1.0 --no-workloads", commands)
        self.assertIn('board_perf.py --boards "$PERF_BOARDS" --require-all', commands)
        self.assertNotIn("--modes", commands)
        self.assertNotIn("--update", commands)
        self.assertNotIn("espup", commands)
        self.assertTrue(all(step.get("continue-on-error") is not True for step in job["steps"]))

    def test_full_measurement_is_separate_and_fail_closed(self):
        jobs = workflow("core-perf.yml")["jobs"]
        self.assertIn("outputs.full_required", jobs["board-throughput"]["if"])
        step = jobs["perf-verdict"]["steps"][-1]
        self.assertEqual(step["env"]["REQUIRED"], "${{ needs.perf-scope.outputs.full_required }}")
        self.assertIn("pr_perf_verdict.py", step["run"])
        self.assertIn("always()", jobs["perf-verdict"]["if"])


class RealSelectorControls(HistoryControls):
    def run_selector(self, changed, label="false", invalid_source=False, workflow_file="core-perf-pr.yml"):
        self.run_git("checkout", "-b", "pr")
        self.write(changed, "changed\n")
        self.run_git("add", ".")
        self.run_git("commit", "-m", "PR")
        head = self.sha()
        self.run_git("checkout", "main")
        self.run_git("merge", "--no-ff", "pr", "-m", "tested merge")
        tested = self.sha()
        self.write("scripts/perf/pr_hotpath_gate.py", (ROOT / "scripts/perf/pr_hotpath_gate.py").read_text())
        output = self.repo / "github-output"
        select = next(s for s in workflow(workflow_file)["jobs"]["perf-scope"]["steps"] if s.get("id") == "select")
        env = dict(os.environ, EVENT="pull_request", REPOSITORY="w1ne/labwired-core", HEAD_REF="fix/pr", TESTED=tested,
                   BASE="bad" if invalid_source else self.initial, HEAD=head, PERF_LABELED=label, GITHUB_OUTPUT=str(output))
        proc = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", select["run"]], env=env, capture_output=True, text=True)
        fields = dict(line.split("=", 1) for line in output.read_text().splitlines()) if output.exists() else {}
        return proc, fields

    def test_missing_perf_label_cannot_disable_runtime_measurement(self):
        proc, fields = self.run_selector("crates/core/src/lib.rs")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(fields["required"], "true")

    def test_full_lane_cannot_override_mandatory_scope_without_a_label(self):
        proc, fields = self.run_selector("crates/wasm/src/stepping.rs", workflow_file="core-perf.yml")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(fields, {"required": "true", "full_required": "false"})

    def test_perf_label_requests_additional_full_measurement(self):
        proc, fields = self.run_selector("crates/core/src/lib.rs", label="true", workflow_file="core-perf.yml")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(fields, {"required": "true", "full_required": "true"})

    def test_wasm_only_change_requires_measurement(self):
        proc, fields = self.run_selector("crates/wasm/src/stepping.rs")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(fields["required"], "true")

    def test_source_error_writes_no_scope_success(self):
        proc, fields = self.run_selector("README.md", invalid_source=True)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(fields, {})

    def test_docs_only_reaches_explicit_skip_then_required_aggregate_success(self):
        proc, fields = self.run_selector("README.md")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(fields["required"], "false")
        self.assertEqual(verdict.check("success", fields["required"], "skipped", "false"), "not-required")
        step = workflow("core-ci.yml")["jobs"]["pr-core-feature-off"]["steps"][0]
        proc = subprocess.run(["bash", "-e", "-c", step["run"]], env=dict(os.environ, DEFAULT_MEMBERS_RESULT="success", FEATURE_OFF_RESULT="success", NGSPICE_RESULT="success", PERF_RESULT="success"), capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
