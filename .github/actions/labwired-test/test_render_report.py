#!/usr/bin/env python3
"""Regression coverage for the self-contained LabWired CI report renderer."""

from __future__ import annotations

import json
import hashlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


RENDERER = Path(__file__).with_name("render_report.py")
SCRIPT_CONTENTS = "name: firmware test\n"
UART_LIMIT_BYTES = 64 * 1024


def parse_github_output(contents: str) -> dict[str, str]:
    outputs: dict[str, str] = {}
    lines = contents.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        if "<<" in line:
            key, delimiter = line.split("<<", 1)
            index += 1
            value_lines: list[str] = []
            while index < len(lines) and lines[index] != delimiter:
                value_lines.append(lines[index])
                index += 1
            outputs[key] = "\n".join(value_lines)
        elif "=" in line:
            key, value = line.split("=", 1)
            outputs[key] = value
        index += 1
    return outputs


class RenderReportTest(unittest.TestCase):
    def render(
        self,
        result_contents: str | None,
        uart_contents: str | bytes | None = "TESTER_REQ_22 <tag>\n",
        environment_updates: dict[str, str] | None = None,
        output_directory_name: str = "reports",
    ) -> tuple[dict[str, str], str, str, str, str]:
        with tempfile.TemporaryDirectory() as temp_dir:
            directory = Path(temp_dir)
            result_json = directory / "result.json"
            uart_log = directory / "uart.log"
            script = directory / "firmware-test.yaml"
            output_directory = directory / output_directory_name
            summary_md = output_directory / "summary.md"
            report_html = output_directory / "report.html"
            github_output = directory / "github-output"

            if result_contents is not None:
                result_json.write_text(result_contents, encoding="utf-8")
            if isinstance(uart_contents, bytes):
                uart_log.write_bytes(uart_contents)
            elif uart_contents is not None:
                uart_log.write_text(uart_contents, encoding="utf-8")
            script.write_text(SCRIPT_CONTENTS, encoding="utf-8")

            environment = os.environ.copy()
            environment.update(
                {
                    "LABWIRED_RUN_URL": "https://github.com/w1ne/udslib/actions/runs/42",
                    "LABWIRED_SOURCE_REVISION": "abc123",
                    "LABWIRED_RELEASE_VERSION": "v0.18.0",
                    "LABWIRED_SCRIPT": str(script),
                }
            )
            if environment_updates is not None:
                environment.update(environment_updates)
            subprocess.run(
                [
                    "python3",
                    str(RENDERER),
                    str(result_json),
                    str(uart_log),
                    str(summary_md),
                    str(report_html),
                    str(github_output),
                ],
                check=True,
                env=environment,
            )

            outputs = parse_github_output(github_output.read_text(encoding="utf-8"))
            return (
                outputs,
                summary_md.read_text(encoding="utf-8"),
                report_html.read_text(encoding="utf-8"),
                str(summary_md),
                str(report_html),
            )

    def test_renders_passing_result_with_safe_provenance_and_uart(self) -> None:
        outputs, summary, report_html, summary_path, report_path = self.render(
            json.dumps(
                {
                    "status": "pass",
                    "stop_reason": "assertions_passed",
                    "cycles": 42,
                    "steps_executed": 84,
                    "instructions": 84,
                    "assertions": [
                        {
                            "assertion": {"uart_contains": "<assertion>"},
                            "passed": True,
                        }
                    ],
                }
            ),
            environment_updates={"LABWIRED_SOURCE_REVISION": "abc<123>"},
        )

        self.assertEqual(outputs["status"], "pass")
        self.assertEqual(outputs["summary_md"], summary_path)
        self.assertEqual(outputs["report_html"], report_path)
        self.assertIn("## LabWired test", summary)
        self.assertIn("Assertions: `1` passed, `0` failed", summary)
        self.assertIn("&lt;tag&gt;", report_html)
        self.assertIn("https://github.com/w1ne/udslib/actions/runs/42", report_html)
        self.assertIn("sha256", report_html)
        self.assertIn(hashlib.sha256(SCRIPT_CONTENTS.encode("utf-8")).hexdigest(), report_html)
        self.assertIn("Steps Executed", report_html)
        self.assertIn("<table>", report_html)
        self.assertIn("&lt;assertion&gt;", report_html)
        self.assertIn("abc&lt;123&gt;", report_html)

    def test_malformed_or_missing_artifacts_still_produce_unknown_reports(self) -> None:
        for result_contents, uart_contents in (("{ this is not JSON", "UART\n"), (None, None)):
            with self.subTest(result_contents=result_contents):
                outputs, summary, report_html, summary_path, report_path = self.render(
                    result_contents, uart_contents
                )

                self.assertEqual(outputs["status"], "unknown")
                self.assertEqual(outputs["summary_md"], summary_path)
                self.assertEqual(outputs["report_html"], report_path)
                self.assertIn("unknown", summary)
                self.assertIn("unknown", report_html)
        self.assertIn("[UART transcript unavailable]", report_html)

    def test_uart_transcript_is_capped_and_marked(self) -> None:
        _, _, report_html, _, _ = self.render(
            json.dumps({"status": "pass", "assertions": []}),
            b"A" * UART_LIMIT_BYTES + b"AFTER_LIMIT",
        )

        self.assertIn("[UART transcript truncated after 65536 bytes]", report_html)
        self.assertNotIn("AFTER_LIMIT", report_html)

    def test_github_output_does_not_allow_path_newline_injection(self) -> None:
        outputs, _, _, _, _ = self.render(
            json.dumps({"status": "pass", "assertions": []}),
            output_directory_name="reports\ninjected-output=attacker-value",
        )

        self.assertEqual(outputs["status"], "pass")
        self.assertIn("summary_md", outputs)
        self.assertIn("report_html", outputs)
        self.assertNotIn("injected-output", outputs)

    def test_markdown_provenance_uses_a_safe_code_delimiter(self) -> None:
        run_url = "`[unexpected](https://attacker.invalid)`"
        _, summary, _, _, _ = self.render(
            json.dumps({"status": "pass", "assertions": []}),
            environment_updates={"LABWIRED_RUN_URL": run_url},
        )

        self.assertIn(f"Run: `` {run_url} ``", summary)


if __name__ == "__main__":
    unittest.main()
