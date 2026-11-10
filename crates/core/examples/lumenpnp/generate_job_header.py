#!/usr/bin/env python3
"""Build-time data adapter, NOT a new MCP or simulator.

Converts an assembly job into a firmware constant table. The native Core
runner independently reads the same JSON and checks its SHA-256 in UART.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

def validated(job_path: Path, contract_path: Path):
    raw = job_path.read_bytes()
    job = json.loads(raw)
    contract = json.loads(contract_path.read_text())
    if not isinstance(job, dict) or set(job) != {"schema", "placements"} or job["schema"] != "labwired.assembly-job.v1":
        raise ValueError("invalid job schema")
    placements = job["placements"]
    if not isinstance(placements, list) or not (1 <= len(placements) <= 16):
        raise ValueError("assembly job must contain 1..16 placements")
    pockets = {p["index"]: p for p in contract["tray"]["pockets"]}
    refs = set()
    used = set()
    for p in placements:
        if not isinstance(p, dict) or set(p) - {"ref", "head", "source_pocket", "retry_pocket", "x_mm", "y_mm", "rotation_deg"}:
            raise ValueError("invalid placement fields")
        if not isinstance(p.get("ref"), str) or not p["ref"] or p["ref"] in refs:
            raise ValueError("duplicate or empty reference")
        refs.add(p["ref"])
        if p.get("head") not in ("A", "B"):
            raise ValueError("unsupported head")
        primary = p.get("source_pocket")
        retry = p.get("retry_pocket", primary)
        if type(primary) is not int or type(retry) is not int or primary not in pockets or retry not in pockets:
            raise ValueError("unknown source pocket")
        if not pockets[retry]["populated"]:
            raise ValueError("fallback pocket must be populated")
        if primary != retry and pockets[primary]["populated"]:
            raise ValueError("fallback only allowed for an empty primary pocket")
        if retry in used:
            raise ValueError("the same source part cannot be placed twice")
        used.add(retry)
        for key in ("x_mm", "y_mm"):
            if type(p.get(key)) is not int:
                raise ValueError(key + " must be integer millimeters")
        x, y = p["x_mm"], p["y_mm"]
        board = contract["board"]["bounds"]
        size = pockets[retry]["size"]
        if not (board[0][0]+size[0]/2 <= x <= board[1][0]-size[0]/2 and
                board[0][1]+size[1]/2 <= y <= board[1][1]-size[1]/2):
            raise ValueError("placement footprint outside PCB")
        if type(p.get("rotation_deg")) not in (int, float) or not math.isfinite(p["rotation_deg"]):
            raise ValueError("nonfinite rotation")
        steps = p["rotation_deg"] / 0.225
        if abs(steps - round(steps)) > 1e-5 or not (-180 <= p["rotation_deg"] <= 180):
            raise ValueError("rotation must fit 0.225-degree motor steps")
        head_x = x - (48 if p["head"] == "B" else 0)
        if head_x < 0 or head_x > 400 or y < 0 or y > 400:
            raise ValueError("axis position outside travel")
    return raw, placements

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--job", required=True, type=Path)
    parser.add_argument("--geometry", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args=parser.parse_args()
    raw, placements=validated(args.job, args.geometry)
    digest=hashlib.sha256(raw).hexdigest()
    rows=[]
    for p in placements:
        rows.append("{%d,%d,%d,%d,%d,%d}" % (
            1 if p["head"]=="B" else 0,
            p["source_pocket"],p.get("retry_pocket",p["source_pocket"]),
            p["x_mm"],p["y_mm"],int(round(p["rotation_deg"]/0.225))
        ))
    text = (
        "/* Generated from user assembly job. Do not edit. */\n"
        "#ifndef LABWIRED_ASSEMBLY_JOB_H\n#define LABWIRED_ASSEMBLY_JOB_H\n"
        "typedef struct { unsigned head, primary_pocket, retry_pocket; int x_mm, y_mm, rotation_steps; } PlacementTask;\n"
        "#define ASSEMBLY_JOB_COUNT %d\n" % len(placements)
        + 'static const char ASSEMBLY_JOB_SHA256[] = "%s";\n' % digest
        + "static const PlacementTask ASSEMBLY_JOB[] = {\n  "
        + ",\n  ".join(rows)
        + "\n};\n#endif\n"
    )
    args.out.write_text(text)
    print("JOB_VALIDATED", len(placements),digest)

if __name__ == "__main__":
    main()
