# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

import json
import os
import sys


def main() -> int:
    if len(sys.argv) != 4:
        raise SystemExit("Usage: summary.py <result.json> <summary.md> <github_output>")

    result_path, summary_path, github_output = sys.argv[1:]

    status = "unknown"
    stop_reason = "unknown"
    message = None
    steps_executed = None
    cycles = None
    instructions = None
    firmware_hash = None
    assertions = []

    try:
        with open(result_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        status = str(data.get("status", "unknown"))
        stop_reason = str(data.get("stop_reason", "unknown"))
        message = data.get("message")
        steps_executed = data.get("steps_executed")
        cycles = data.get("cycles")
        instructions = data.get("instructions")
        firmware_hash = data.get("firmware_hash")
        assertions = data.get("assertions", []) or []
    except Exception:
        data = None

    passed = sum(1 for a in assertions if a.get("passed") is True)
    failed = sum(1 for a in assertions if a.get("passed") is False)

    lines: list[str] = []
    lines.append("## LabWired test")
    lines.append("")
    lines.append(f"- Status: **{status}**")
    lines.append(f"- Stop reason: `{stop_reason}`")
    if message:
        msg = str(message)
        if len(msg) > 1000:
            msg = msg[:1000] + "…"
        lines.append("- Message:")
        lines.append("")
        lines.append("```")
        lines.append(msg.rstrip())
        lines.append("```")
    if steps_executed is not None:
        lines.append(f"- Steps executed: `{steps_executed}`")
    if cycles is not None:
        lines.append(f"- Cycles: `{cycles}`")
    if instructions is not None:
        lines.append(f"- Instructions: `{instructions}`")
    if firmware_hash and stop_reason != "config_error":
        lines.append(f"- Firmware hash: `{firmware_hash}`")
    if assertions:
        lines.append(f"- Assertions: `{passed}` passed, `{failed}` failed")
    lines.append("")
    lines.append("Artifacts:")
    lines.append("")
    artifacts_dir = os.path.dirname(result_path)
    lines.append(f"- `{artifacts_dir}/result.json`")
    lines.append(f"- `{artifacts_dir}/uart.log`")
    lines.append(f"- `{artifacts_dir}/junit.xml`")

    os.makedirs(os.path.dirname(summary_path), exist_ok=True)
    with open(summary_path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines).rstrip() + "\n")

    with open(github_output, "a", encoding="utf-8") as f:
        f.write(f"summary_md={summary_path}\n")
        f.write(f"status={status}\n")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
