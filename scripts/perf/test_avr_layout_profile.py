# SPDX-License-Identifier: MIT
"""Pure C9 admission controls; no compiler, profiler or guest is executed."""

import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import avr_layout_profile as producer


def original(mode="step", steps=200_000, irefs=250_000_000):
    suffix = ", batched" if mode == "batch" else ""
    batch = (
        (
            f"[batched] instructions={steps} batches=200 "
            f"steps_per_batch={steps / 200:.2f} tick_interval=1024\n"
        )
        if suffix
        else ""
    )
    stderr = (
        batch
        + f"labwired-cli run (avr{suffix}): reached --max-steps {steps}; pc=0x00000002\n"
        f"==17== I   refs: {irefs:,}\n"
    )
    profile = f"# callgrind format\nversion: 1\nevents: Ir\nsummary: {irefs}\n"
    return stderr, profile


class CounterAdmission(unittest.TestCase):
    def test_complete_original_witnesses(self):
        for mode in ["step", "batch"]:
            for steps in [producer.LOW, producer.HIGH]:
                with self.subTest(mode=mode, steps=steps):
                    parsed = producer.parse_run(*original(mode, steps), mode, steps)
                    self.assertEqual(parsed["irefs"], 250_000_000)
                    self.assertEqual(parsed["requestedSteps"], steps)
        # Width one is a real observation, not an invented diagnostic waiver or
        # a threshold replacing the existing Core Perf contract.
        stderr, profile = original("batch")
        stderr = stderr.replace("batches=200", "batches=200000").replace(
            "steps_per_batch=1000.00", "steps_per_batch=1.00"
        )
        self.assertEqual(
            producer.parse_run(stderr, profile, "batch", producer.LOW)["stepsPerBatch"],
            1,
        )

    def test_missing_and_duplicate_original_witnesses_fail(self):
        for mode in ["step", "batch"]:
            stderr, profile = original(mode)
            for text in ["I   refs:", "reached --max-steps"]:
                with self.subTest(mode=mode, missing=text):
                    with self.assertRaises(ValueError):
                        producer.parse_run(
                            stderr.replace(text, "absent:"), profile, mode, producer.LOW
                        )
            for changed in [stderr + stderr, stderr.replace("200000;", "199999;")]:
                with self.assertRaises(ValueError):
                    producer.parse_run(changed, profile, mode, producer.LOW)
            for changed in [
                profile.replace("summary:", "absent:"),
                profile + "summary: 250000000\n",
                profile.replace("250000000", "249999999"),
                profile.replace("events: Ir", "events: Ir Dr"),
            ]:
                with self.assertRaises(ValueError):
                    producer.parse_run(stderr, changed, mode, producer.LOW)

    def test_wrong_path_cannot_qualify_the_other_mode(self):
        for observed, claimed in [("step", "batch"), ("batch", "step")]:
            with self.assertRaises(ValueError):
                producer.parse_run(*original(observed), claimed, producer.LOW)

    def test_batch_retirement_and_ratio_cuts(self):
        stderr, profile = original("batch")
        cuts = [
            ("instructions=200000", "instructions=199999"),
            ("batches=200", "batches=0"),
            ("batches=200", "batches=200001"),
            ("steps_per_batch=1000.00", "steps_per_batch=0.00"),
            ("steps_per_batch=1000.00", "steps_per_batch=999.00"),
            ("tick_interval=1024", "tick_interval=0"),
        ]
        for before, after in cuts:
            with self.subTest(cut=after):
                with self.assertRaises(ValueError):
                    producer.parse_run(
                        stderr.replace(before, after), profile, "batch", producer.LOW
                    )
        with self.assertRaises(ValueError):
            producer.parse_run(
                stderr + stderr.splitlines()[0] + "\n", profile, "batch", producer.LOW
            )

    def test_nonpositive_counts_and_unknown_modes_fail(self):
        for count in [0, -1]:
            with self.assertRaises(ValueError):
                producer.parse_run(*original(irefs=count), "step", producer.LOW)
        with self.assertRaises(ValueError):
            producer.parse_run(*original(), "unknown", producer.LOW)

    def test_profiling_command_has_closed_bounds_and_no_baseline_mutation(self):
        for mode in ["step", "batch"]:
            argv = producer.profile_command(
                Path("cli"),
                Path("chip"),
                Path("firmware"),
                Path("profile"),
                mode,
                producer.LOW,
            )
            self.assertEqual(argv[-1] == "--batched", mode == "batch")
            self.assertIn("--collect-jumps=yes", argv)
            self.assertIn("--dump-instr=yes", argv)
            self.assertNotIn("--update", argv)
        for mode, steps in [
            ("unknown", producer.LOW),
            ("step", 0),
            ("batch", producer.HIGH + 1),
        ]:
            with self.assertRaises(ValueError):
                producer.profile_command(
                    Path("cli"),
                    Path("chip"),
                    Path("firmware"),
                    Path("profile"),
                    mode,
                    steps,
                )


class OriginalRetention(unittest.TestCase):
    def test_failed_command_keeps_both_streams_and_never_retries(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            commands = producer.Commands(output)

            def failure(argv, **kwargs):
                kwargs["stdout"].write(b"partial original stdout\n")
                kwargs["stderr"].write(b"original failure\n")
                return subprocess.CompletedProcess(argv, 7)

            with (
                patch.object(producer.subprocess, "run", side_effect=failure) as run,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                with self.assertRaisesRegex(RuntimeError, "exited 7"):
                    commands.run("manufactured", ["never-executed"], output)
            self.assertEqual(run.call_count, 1)
            self.assertEqual(
                (output / "01-manufactured.stdout").read_bytes(),
                b"partial original stdout\n",
            )
            self.assertEqual(
                (output / "01-manufactured.stderr").read_bytes(), b"original failure\n"
            )
            rows = [
                json.loads(row)
                for row in (output / "commands.jsonl").read_text().splitlines()
            ]
            self.assertEqual([row["phase"] for row in rows], ["before", "after"])
            self.assertEqual(rows[-1]["returncode"], 7)

    def test_failed_production_inventory_preserves_partial_and_refuses_resume(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "fresh"

            def failure(source, target, variant):
                (target / "partial-profile").write_bytes(b"retained original\n")
                raise RuntimeError("manufactured boundary failure")

            argv = [
                "profile",
                "--source",
                directory,
                "--output",
                str(output),
                "--variant",
                "current",
            ]
            with (
                patch.object(producer, "produce", side_effect=failure),
                patch.object(producer.sys, "argv", argv),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(producer.main(), 1)
                with self.assertRaises(FileExistsError):
                    producer.main()
            result = json.loads((output / "result.json").read_text())
            self.assertTrue(result["failed"])
            self.assertEqual(result["error"], "manufactured boundary failure")
            inventory = json.loads((output / "inventory.json").read_text())
            self.assertEqual(
                inventory["partial-profile"]["bytes"], len(b"retained original\n")
            )
            self.assertIn("result.json", inventory)


if __name__ == "__main__":
    unittest.main()
