#!/usr/bin/env python3
"""Controls for the public open/closed model-rating table."""

from pathlib import Path
import json
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SIDECAR = ROOT / "docs/coverage/closed-model-ratings.json"


def test_closed_model_ratings_are_public_and_source_free():
    raw = json.loads(SIDECAR.read_text())
    assert raw["schema"] == 1
    assert raw["models"]
    for model in raw["models"]:
        assert model["visibility"] == "closed"
        assert (ROOT / model["public_doc"]).is_file()
        blob = json.dumps(model).lower()
        for forbidden in ("labwired-ip", "/crates/", "target/release"):
            assert forbidden not in blob, (model["name"], forbidden)
        assert not any(
            isinstance(value, str) and (value.startswith("/") or value.startswith("~"))
            for value in model.values()
        )
        assert model["conformance"]["level"] in (0, 1, 2)
        assert model["perf"]["step_ir"] > 0
        assert model["perf"]["measured_by"] == "labwired-pro"


def test_generated_table_is_current():
    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts/generate_model_ratings.py"), "--check"],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    table = (ROOT / "docs/coverage/model-ratings.md").read_text()
    assert "| k32w041z (closed) | closed | L2 |" in table
    assert "labwired-ip" not in table
