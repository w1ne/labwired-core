#!/usr/bin/env python3
"""Generate the public model-rating table for open and closed models.

Open rows come from the measured chip-conformance board and the committed
step/batch baselines. Closed rows come from `closed-model-ratings.json`: a
public rating record only. The closed model, chip YAML and source are never
read by this script and must never appear in the generated output.

Run `--check` in CI; it fails when the committed table is stale or a closed
record carries a private path.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import json
import sys

ROOT = Path(__file__).resolve().parents[1]
CONFORMANCE = ROOT / "docs/coverage/chip-conformance.md"
BASELINES = ROOT / "scripts/perf/baselines.json"
CLOSED = ROOT / "docs/coverage/closed-model-ratings.json"
OUT = ROOT / "docs/coverage/model-ratings.md"
MAX_CLOSED_RECORDS = 64


def fail(message: str) -> None:
    raise ValueError(message)


def open_rows() -> list[dict]:
    text = CONFORMANCE.read_text()
    rows: list[dict] = []
    in_table = False
    for line in text.splitlines():
        if line.startswith("| Chip | Level |"):
            in_table = True
            continue
        if not in_table:
            continue
        if not line.startswith("|"):
            break
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 7 or cells[0] in {"---", "Chip"}:
            continue
        if set(cells[0]) <= {"-", ":"}:
            continue
        name, level, estate, peripherals, reg, excluded, behavior = cells
        if level.startswith("**L"):
            level = level.strip("*").lstrip("L")
        rows.append(
            {
                "name": name,
                "level": level,
                "estate": estate,
                "peripherals": peripherals,
                "reg_match": reg,
                "excluded": excluded,
                "behavior": behavior,
                "visibility": "open",
            }
        )
    if not rows:
        fail("no open-model rows found in the conformance board")
    return rows


def load_baselines() -> dict:
    if not BASELINES.exists():
        fail(f"missing {BASELINES.relative_to(ROOT)}")
    raw = json.loads(BASELINES.read_text())
    if not isinstance(raw, dict):
        fail("baselines.json must be an object")
    return raw


def load_closed() -> list[dict]:
    if not CLOSED.exists():
        fail(f"missing {CLOSED.relative_to(ROOT)}")
    raw = json.loads(CLOSED.read_text())
    if raw.get("schema") != 1:
        fail("closed-model-ratings.json: schema must be 1")
    models = raw.get("models")
    if not isinstance(models, list) or not models:
        fail("closed-model-ratings.json: models must be a non-empty list")
    if len(models) > MAX_CLOSED_RECORDS:
        fail("closed-model-ratings.json: too many records")
    seen = set()
    for model in models:
        name = model.get("name")
        if not isinstance(name, str) or not name:
            fail("closed model: name is required")
        if name in seen:
            fail(f"closed model: duplicate {name}")
        seen.add(name)
        if model.get("visibility") != "closed":
            fail(f"{name}: visibility must be 'closed'")
        doc = model.get("public_doc")
        if not isinstance(doc, str) or not (ROOT / doc).is_file():
            fail(f"{name}: public_doc must name an existing public board doc")
        conformance = model.get("conformance")
        perf = model.get("perf")
        if not isinstance(conformance, dict) or not isinstance(perf, dict):
            fail(f"{name}: conformance and perf objects are required")
        for key in ("level", "estate", "peripherals", "behavior_gate"):
            if key not in conformance:
                fail(f"{name}: conformance.{key} is required")
        for key in ("step_ir", "batch_ir", "rtx", "measured_by", "measured_at"):
            if key not in perf:
                fail(f"{name}: perf.{key} is required")
        _reject_private_strings(model, name)
    return models


def _reject_private_strings(value: object, name: str) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            _reject_private_strings(key, name)
            _reject_private_strings(item, name)
    elif isinstance(value, list):
        for item in value:
            _reject_private_strings(item, name)
    elif isinstance(value, str):
        lowered = value.lower()
        if "labwired-ip" in lowered or "labwired_core" in lowered:
            fail(f"{name}: private model path/repository in public rating")
        if value.startswith("/") or value.startswith("~"):
            fail(f"{name}: absolute path in public rating")
        if "/crates/" in lowered or "target/release" in lowered:
            fail(f"{name}: build path in public rating")


def doc_link(name: str) -> str:
    path = ROOT / f"docs/boards/{name}.md"
    if path.is_file():
        return f"[doc](docs/boards/{name}.md)"
    return "—"


def render() -> str:
    baselines = load_baselines()
    closed = load_closed()
    lines = [
        "# Model ratings",
        "",
        "GENERATED by `scripts/generate_model_ratings.py` — do not hand-edit.",
        "Run the script to refresh; `--check` fails when this file is stale.",
        "",
        "All models use the same rating fields. Open models carry their measured",
        "step/batch baselines and the CI real-time gate; closed models carry the",
        "same fields but never their source, chip YAML, binary or build paths.",
        "",
        "| Model | Visibility | Conformance | Peripherals | Step Ir/step | Batch Ir/step | RTx | Public docs |",
        "| --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for row in open_rows():
        name = row["name"]
        perf = baselines.get(name, {})
        step = perf.get("step", "—")
        batch = perf.get("batch", "—")
        lines.append(
            f"| {name} | open | L{row['level']} | {row['peripherals']} | "
            f"{step} | {batch} | ≥1× (CI) | {doc_link(name)} |"
        )
    for model in closed:
        name = model["name"]
        c = model["conformance"]
        p = model["perf"]
        rtx = p["rtx"]
        rtx_text = f"{rtx:.2f}× FAIL" if p.get("rtx_ok") is False else f"{rtx:.2f}×"
        batch = p["batch_ir"]
        if p.get("batch_ok") is False:
            batch = f"{batch} (tick {p.get('batch_tick_interval', '?')})"
        lines.append(
            f"| {name} (closed) | closed | L{c['level']} | {c['peripherals']} | "
            f"{p['step_ir']} | {batch} | {rtx_text} | {doc_link(model['public_doc'].removeprefix('docs/boards/').removesuffix('.md'))} |"
        )
    lines += [
        "",
        "Open-model RTx is a CI receipt; the committed open perf rating is step/batch.",
        "Closed-model RTx is measured by the private build and published here as a",
        "rating only. `FAIL` means below the open-model real-time floor.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        generated = render()
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    if args.check:
        if not OUT.exists():
            print(f"error: {OUT.relative_to(ROOT)} is missing", file=sys.stderr)
            return 1
        if OUT.read_text() != generated:
            print(
                f"error: {OUT.relative_to(ROOT)} is stale; run "
                "python3 scripts/generate_model_ratings.py",
                file=sys.stderr,
            )
            return 1
        print(f"ok: {OUT.relative_to(ROOT)} is current")
        return 0
    OUT.write_text(generated)
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
