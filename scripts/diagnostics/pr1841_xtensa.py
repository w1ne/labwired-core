"""Temporary PR 1841 diagnostic. Its custom guest is not an acceptance fixture."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys

BASE = "5136a4ddf78132f5b411972911042c91d5a67121"
HEAD = "6ec1cfbb24c4230df347d3790fcae6b59172356d"
LOW, HIGH = 200_000, 1_200_000


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def guest(path: Path) -> None:
    # The repository's xtensa_exec.rs helpers encode these three instructions.
    # movi a2,0; loop: addi a2,a2,1; j loop. No peripheral accesses.
    code = b"".join(word.to_bytes(3, "little") for word in (0x00A022, 0x01C222, 0xFFFE46))
    entry, offset = 0x400D0000, 0x1000
    ident = b"\x7fELF" + bytes((1, 1, 1, 0)) + bytes(8)
    header = ident + struct.pack("<HHIIIIIHHHHHH", 2, 94, 1, entry, 52, 0, 0, 52, 32, 1, 40, 0, 0)
    segment = struct.pack("<IIIIIIII", 1, offset, entry, entry, len(code), len(code), 5, 0x1000)
    path.write_bytes((header + segment).ljust(offset, b"\0") + code)


def profile(label: str, source: Path, expected: str, out: Path) -> None:
    source, out = source.resolve(), out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
    if actual != expected:
        raise RuntimeError(f"Unexpected source: {actual} != {expected}")
    dirty = subprocess.check_output(["git", "status", "--porcelain"], cwd=source, text=True).strip()
    if dirty:
        raise RuntimeError(f"Source is dirty: {dirty}")
    binary = Path(os.environ["CARGO_TARGET_DIR"]) / "release/labwired"
    fixture = out / "diagnostic-loop.elf"
    guest(fixture)
    record = {
        "label": label,
        "source_sha": actual,
        "binary_sha256": digest(binary),
        "fixture_sha256": digest(fixture),
        "fixture_kind": "custom diagnostic loop, not the canonical throughput acceptance fixture",
        "rustc": subprocess.check_output(["rustc", "-Vv"], cwd=source, text=True),
        "feature_set": "event-scheduler",
        "profile": "unchanged workspace release profile",
        "mode": "step",
        "gate_or_baseline_changed": False,
        "samples": [],
    }
    env = dict(os.environ, LABWIRED_ARM_SINGLE_STEP="1")
    for steps in (LOW, HIGH):
        raw = out / f"{label}-{steps}.callgrind"
        cmd = ["valgrind", "--tool=callgrind", f"--callgrind-out-file={raw}",
               "--cache-sim=no", "--branch-sim=no", str(binary), "run",
               "--chip", "configs/chips/esp32.yaml", "--firmware", str(fixture),
               "--max-steps", str(steps)]
        run = subprocess.run(cmd, cwd=source, env=env, capture_output=True, text=True, timeout=240)
        (out / f"{label}-{steps}.stdout.txt").write_text(run.stdout)
        (out / f"{label}-{steps}.stderr.txt").write_text(run.stderr)
        match = re.search(r"I\s+refs:\s+([\d,]+)", run.stderr)
        if run.returncode or not match:
            raise RuntimeError(f"{label} {steps}: exit {run.returncode}; no valid completed profile")
        text = run.stdout + run.stderr
        completion = f"reached --max-steps {steps}; pc=0x400d0006"
        if completion not in text or "[batched]" in text:
            raise RuntimeError(f"{label} {steps}: did not prove the requested step count and loop PC")
        record["samples"].append({"steps": steps, "ir": int(match.group(1).replace(",", "")), "command": cmd})
        if steps == HIGH:
            for inclusive in ("yes", "no"):
                annotated = subprocess.run(
                    ["callgrind_annotate", "--auto=no", f"--inclusive={inclusive}", "--threshold=99.9", str(raw)],
                    cwd=source, capture_output=True, text=True, check=True, timeout=60)
                (out / f"{label}-inclusive-{inclusive}.txt").write_text(annotated.stdout)
    a, b = record["samples"]
    record["ir_per_step"] = (b["ir"] - a["ir"]) / (HIGH - LOW)
    (out / f"{label}.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2), flush=True)


def compare(out: Path) -> None:
    base = json.loads((out / "base.json").read_text())
    head = json.loads((out / "head.json").read_text())
    if (base["source_sha"], head["source_sha"]) != (BASE, HEAD):
        raise RuntimeError("Comparison did not use the two reviewed immutable sources")
    if base["fixture_sha256"] != head["fixture_sha256"]:
        raise RuntimeError("Guest bytes differ")
    result = {
        "base_sha": BASE, "head_sha": HEAD,
        "base_ir_per_step": base["ir_per_step"],
        "head_ir_per_step": head["ir_per_step"],
        "delta_ir_per_step": head["ir_per_step"] - base["ir_per_step"],
        "acceptance": False,
        "purpose": "Localize generated-code overhead; the normal full gate remains required.",
    }
    (out / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    if sys.argv[1] == "compare":
        compare(Path(sys.argv[2]).resolve())
    else:
        profile(sys.argv[1], Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4]))
