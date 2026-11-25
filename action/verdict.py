#!/usr/bin/env python3
"""Combine circuit ERC/DRC results and firmware simulation results into a single verdict."""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path


def _read_json(path: Path) -> dict | None:
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else None
    except (OSError, json.JSONDecodeError):
        return None


def _write_outputs(outputs: dict[str, str]) -> None:
    gh_out = os.environ.get("GITHUB_OUTPUT", "")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            for k, v in outputs.items():
                fh.write(f"{k}={v}\n")
    else:
        for k, v in outputs.items():
            print(f"{k}={v}")


def main() -> int:
    out_dir = Path(os.environ.get("OUTDIR", "out/labwired"))
    fail_on_erc = os.environ.get("FAIL_ON_ERC", "true").lower() == "true"
    fail_on_drc = os.environ.get("FAIL_ON_DRC", "true").lower() == "true"
    sim_rc_raw = os.environ.get("SIM_RC", "").strip()
    circuit_rc_raw = os.environ.get("CIRCUIT_RC", "").strip()

    circuit = _read_json(out_dir / "circuit-report.json")
    sim = _read_json(out_dir / "result.json")

    erc_checked = False
    erc_errors = 0
    erc_warnings = 0
    drc_checked = False
    drc_errors = 0
    drc_warnings = 0

    if circuit:
        erc = circuit.get("erc") or {}
        drc = circuit.get("drc") or {}
        erc_checked = bool(erc.get("checked"))
        erc_errors = int(erc.get("errors", 0) or 0)
        erc_warnings = int(erc.get("warnings", 0) or 0)
        drc_checked = bool(drc.get("checked"))
        drc_errors = int(drc.get("errors", 0) or 0)
        drc_warnings = int(drc.get("warnings", 0) or 0)

    assertions_passed = 0
    assertions_total = 0
    sim_verdict: str | None = None

    if sim:
        assertions = sim.get("assertions")
        if isinstance(assertions, list):
            assertions_total = len(assertions)
            assertions_passed = sum(
                1 for item in assertions if isinstance(item, dict) and item.get("passed") is True
            )
        status = str(sim.get("status", "")).lower()
        if status == "pass":
            sim_verdict = "pass"
        elif status == "fail":
            sim_verdict = "fail"
        elif status:
            sim_verdict = "error"

    if sim_verdict is None and sim_rc_raw:
        try:
            rc = int(sim_rc_raw)
        except ValueError:
            rc = 2
        sim_verdict = "pass" if rc == 0 else ("fail" if rc == 1 else "error")

    circuit_failed = False
    if circuit_rc_raw and circuit_rc_raw != "0":
        circuit_failed = True
    if fail_on_erc and erc_checked and erc_errors > 0:
        circuit_failed = True
    if fail_on_drc and drc_checked and drc_errors > 0:
        circuit_failed = True

    nothing_ran = not (circuit and circuit.get("checked")) and sim is None and not sim_rc_raw
    if nothing_ran:
        verdict = "error"
    elif sim_verdict == "error":
        verdict = "error"
    elif circuit_failed or sim_verdict == "fail":
        verdict = "fail"
    elif (circuit and circuit.get("checked")) or sim_verdict == "pass":
        verdict = "pass"
    else:
        verdict = "unproven"

    parts: list[str] = []
    if erc_checked:
        parts.append(f"ERC {erc_errors}/{erc_warnings}")
    else:
        parts.append("ERC —")
    if drc_checked:
        parts.append(f"DRC {drc_errors}/{drc_warnings}")
    else:
        parts.append("DRC —")
    if assertions_total > 0 or sim is not None:
        parts.append(f"{assertions_passed}/{assertions_total} firmware assertions passed")
    else:
        parts.append("no firmware assertions configured")

    summary_line = " · ".join(parts)

    _write_outputs({
        "verdict": verdict,
        "summary_line": summary_line,
        "erc_errors": str(erc_errors),
        "erc_warnings": str(erc_warnings),
        "drc_errors": str(drc_errors),
        "drc_warnings": str(drc_warnings),
        "assertions_passed": str(assertions_passed),
        "assertions_total": str(assertions_total),
    })

    print(f"LabWired verdict: {verdict} — {summary_line}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
