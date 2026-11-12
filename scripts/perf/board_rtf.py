#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Realtime factor (RTF) of realistic per-board workloads.

``board_rtx.py`` proves every chip keeps up with its clock on a spin loop with
no attached parts. Users run sketches with displays, keypads and buses, where
peripheral servicing — not the CPU — sets the speed. This module runs one
committed stock-Arduino workload per main board through ``labwired test`` for
a fixed guest time and reports:

    guest_s = cycles / cpu_hz
    rtf     = guest_s / wall_s      (process startup included, like RTx)
    Mcycles/s

Wall clock depends on the runner, so the gate also compares a normalized
index: ``index = best workload RTF / best spin RTx`` of the same chip in the
same run, with the spin scaled to target ~1 s of wall clock. This cancels only a common
host-speed factor; different instruction mixes and spin-only optimizations
can move the ratio. Keep the raw samples and input provenance when comparing.
A workload whose index drops more than ``--rtf-tolerance`` (default 25%)
below ``rtf_baselines.json`` fails. Workload sources, systems and ELFs live
in ``tests/fixtures/rtf/<id>/``.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import platform
import re
import statistics
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "tests" / "fixtures" / "rtf"
BASELINES = Path(__file__).resolve().parent / "rtf_baselines.json"
# Generous on purpose: wall clock on a shared runner moved the index by up to
# ~10% between back-to-back runs of the same build (one spin outlier moved it
# 50%). The deterministic Ir/step gate (board_perf.py) remains the sensitive
# detector; this one catches the big, user-visible slowdowns.
DEFAULT_TOLERANCE = 0.25
# The pinned Pico sketch redraws its counter every 100 ms. In an 8 s run,
# allow five updates (0.5 s) for startup/missed draws: require at least 75
# distinct meaningful frames and a final frame at or after 7.5 s. Inspect
# already records this history; no per-instruction observer is added.
PICO_FRAME_PERIOD_US = 100_000
PICO_FRAME_ALLOWANCE = 5
PICO_MIN_INK_BYTES = 16  # Reject blank/noise frames; the counter's first frame has 24.


@dataclass(frozen=True)
class Workload:
    id: str
    board: str
    chip: str
    cpu_hz: int
    guest_s: float

    @property
    def dir(self) -> Path:
        return FIXTURES / self.id


WORKLOADS = (
    Workload("uno_feeder", "Arduino Uno", "atmega328p", 16_000_000, 2.0),
    Workload("pico_oled_pwm", "Raspberry Pi Pico", "rp2040", 125_000_000, 8.0),
    Workload("c3_i2c_uart", "ESP32-C3 DevKitM-1", "esp32c3", 160_000_000, 0.5),
    Workload("f411_timer_uart", "Black Pill F411", "stm32f411ceu6", 100_000_000, 0.5),
)


def rtf(cycles: int, cpu_hz: int, wall_s: float) -> float:
    if cpu_hz <= 0 or not math.isfinite(wall_s) or wall_s <= 0:
        raise ValueError("cpu_hz and wall_s must be positive")
    return (cycles / cpu_hz) / wall_s


def write_script(path: Path, w: Workload) -> None:
    cycles = round(w.cpu_hz * w.guest_s)
    path.write_text(
        "\n".join(
            [
                'schema_version: "1.0"',
                "inputs:",
                f'  firmware: "{w.dir / "firmware.elf"}"',
                f'  system: "{w.dir / "system.yaml"}"',
                "limits:",
                f"  max_steps: {cycles * 4}",
                f"  max_cycles: {cycles}",
                "  wall_time_ms: 600000",
                "  max_uart_bytes: 1000000",
                "assertions:",
                f'  - uart_contains: "RTF_READY {w.id}"',
                "",
            ]
        )
    )


def rp2040_rom_path() -> Path:
    path = Path(os.environ.get(
        "LABWIRED_RP2040_BOOTROM", str(REPO_ROOT / "crates/core/roms/rp2040/bootrom.bin")
    ))
    # run_once executes in REPO_ROOT, including when an override is relative.
    return (path if path.is_absolute() else REPO_ROOT / path).resolve()


def file_provenance(path: Path) -> dict:
    path = path.resolve()
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
        return {"path": str(path), "sha256": digest.hexdigest(), "bytes": path.stat().st_size}
    except OSError as exc:
        raise RuntimeError(f"cannot record measurement input {path}: {exc}") from exc


def measurement_provenance(cli: Path, workloads: list[Workload]) -> dict:
    def git(*args: str, strip: bool = True) -> str | None:
        result = subprocess.run(
            ["git", *args], cwd=REPO_ROOT, capture_output=True, text=True,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() if strip else result.stdout

    diff = git("diff", "HEAD", "--binary", strip=False)
    doc = {
        "cli": file_provenance(cli),
        "source_checkout": {
            "head": git("rev-parse", "HEAD"),
            "tree": git("rev-parse", "HEAD^{tree}"),
            "tracked_dirty": bool(diff) if diff is not None else None,
            "tracked_diff_sha256": hashlib.sha256(diff.encode()).hexdigest() if diff is not None else None,
            "is_cli_build_proof": False,
        },
        "harness": {name: file_provenance(Path(__file__).with_name(name)) for name in
                    ("board_rtf.py", "board_rtx.py", "rtf_baselines.json")},
        "inputs": {w.id: {
            "firmware": file_provenance(w.dir / "firmware.elf"),
            "system": file_provenance(w.dir / "system.yaml"),
            "sketch": file_provenance(w.dir / f"{w.id}.ino"),
            "chip": file_provenance(REPO_ROOT / "configs/chips" / f"{w.chip}.yaml"),
        } for w in workloads},
    }
    if any(w.chip == "rp2040" for w in workloads):
        doc["rp2040_workload_rom"] = file_provenance(rp2040_rom_path())
    return doc


def pico_frame_progress(w: Workload, result: dict) -> dict:
    """Reject READY-then-stalled runs using the same run's OLED history."""
    def reject(message: str):
        raise RuntimeError(f"{w.id}: OLED progress: {message}")

    inspect = result.get("inspect")
    devices = inspect.get("devices") if isinstance(inspect, dict) else None
    if not isinstance(devices, list) or any(not isinstance(d, dict) for d in devices):
        reject("missing or malformed inspect.devices")
    panels = [d for d in devices if d.get("id") == "oled" or d.get("device_type") == "oled-ssd1306"]
    if len(panels) != 1:
        reject("expected exactly one OLED device; missing or ambiguous attachment")
    panel = panels[0]
    attachment = panel.get("attachment")
    if (panel.get("id") != "oled" or panel.get("device_type") != "oled-ssd1306"
            or panel.get("declared") is not True or not isinstance(attachment, dict)
            or attachment.get("transport") != "i2c" or attachment.get("bus") != "i2c0"
            or type(attachment.get("address")) is not int or attachment["address"] != 0x3C):
        reject("expected declared oled-ssd1306 oled on i2c0 address 0x3c")
    artifacts = panel.get("artifacts")
    if not isinstance(artifacts, list) or any(not isinstance(a, dict) for a in artifacts):
        reject("missing or malformed framebuffer artifacts")
    framebuffers = [a for a in artifacts if a.get("kind") == "framebuffer"]
    if len(framebuffers) != 1 or framebuffers[0].get("id") != "oled":
        reject("expected exactly one framebuffer for oled")
    meta = framebuffers[0].get("meta")
    if (not isinstance(meta, dict) or meta.get("device_id") != "oled"
            or meta.get("format") != "ssd1306_page" or type(meta.get("w")) is not int
            or type(meta.get("h")) is not int or meta["w"] != 128
            or meta["h"] != 64 or meta.get("powered") is not True):
        reject("expected a powered 128x64 ssd1306_page framebuffer for oled")
    frames = meta.get("frames_seen")
    if not isinstance(frames, list) or not frames:
        reject("missing or empty frames_seen history")
    budget_us = round(w.guest_s * 1_000_000)
    minimum_frames = max(1, budget_us // PICO_FRAME_PERIOD_US - PICO_FRAME_ALLOWANCE)
    minimum_last_us = max(0, budget_us - PICO_FRAME_ALLOWANCE * PICO_FRAME_PERIOD_US)
    previous_us = 0
    seen = set()
    meaningful = []
    evidence = []
    for i, frame in enumerate(frames):
        if not isinstance(frame, dict):
            reject(f"frame {i} is not an object")
        stamp, ink, sha = frame.get("t_us"), frame.get("ink_bytes"), frame.get("sha256")
        if type(stamp) is not int or not previous_us < stamp <= budget_us:
            reject(f"frame {i} timestamp must increase and remain within 1..{budget_us} us")
        if type(ink) is not int or not 0 <= ink <= 1024:
            reject(f"frame {i} has invalid ink_bytes")
        if not isinstance(sha, str) or re.fullmatch(r"[0-9a-fA-F]{64}", sha) is None:
            reject(f"frame {i} has invalid SHA256")
        previous_us = stamp
        sha = sha.lower()
        evidence.append({"t_us": stamp, "ink_bytes": ink, "sha256": sha})
        if ink >= PICO_MIN_INK_BYTES and sha not in seen:
            meaningful.append(evidence[-1])
            seen.add(sha)
    last_us = meaningful[-1]["t_us"] if meaningful else None
    if len(meaningful) < minimum_frames or last_us < minimum_last_us:
        reject(f"need >= {minimum_frames} unique meaningful frames and last >= {minimum_last_us} us; "
               f"observed {len(meaningful)}, last={last_us}")
    return {
        "device": "oled", "attachment": attachment, "width": 128, "height": 64,
        "powered": True, "frame_period_us": PICO_FRAME_PERIOD_US,
        "minimum_unique_frames": minimum_frames, "minimum_last_t_us": minimum_last_us,
        "minimum_ink_bytes": PICO_MIN_INK_BYTES, "budget_us": budget_us,
        "unique_meaningful_frames": len(meaningful),
        "first_t_us": meaningful[0]["t_us"], "last_t_us": last_us, "frames": evidence,
    }


def run_once(cli: Path, w: Workload) -> dict:
    """One timed run; raises unless the marker printed and the budget was spent."""
    with tempfile.TemporaryDirectory(prefix=f"rtf-{w.id}-") as tmp:
        tmp_path = Path(tmp)
        script = tmp_path / "test.yaml"
        write_script(script, w)
        env = dict(os.environ)
        env["LABWIRED_RP2040_BOOTROM"] = str(rp2040_rom_path())
        start = time.perf_counter()
        proc = subprocess.run(
            [str(cli.resolve()), "test", "--script", str(script), "--output-dir",
             str(tmp_path / "out"), "--no-uart-stdout"],
            capture_output=True, text=True, env=env, cwd=REPO_ROOT,
        )
        wall = time.perf_counter() - start
        try:
            raw = (tmp_path / "out" / "result.json").read_bytes()
            result = json.loads(raw)
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise RuntimeError(f"{w.id}: no result.json ({exc}):\n{proc.stderr[-1500:]}")
    sample = parse_result(w, result, wall, proc.returncode)
    sample["result_sha256"] = hashlib.sha256(raw).hexdigest()
    sample["firmware_sha256"] = result.get("firmware_hash")
    return sample


def parse_result(w: Workload, result: dict, wall: float, returncode: int) -> dict:
    want = round(w.cpu_hz * w.guest_s)
    if not isinstance(result, dict):
        raise RuntimeError(f"{w.id}: malformed result.json; expected an object")
    if returncode != 0 or result.get("status") != "pass":
        raise RuntimeError(
            f"{w.id}: run did not pass (exit {returncode}, status "
            f"{result.get('status')!r}, stop {result.get('stop_reason')!r})"
        )
    # `max_cycles` is checked against the machine clock, so a run that stopped
    # on it simulated exactly the budget. Any other stop would report an
    # inflated RTF for less guest time, so it is refused.
    if result.get("stop_reason") != "max_cycles":
        raise RuntimeError(f"{w.id}: stopped on {result.get('stop_reason')!r}, not max_cycles")
    sample = {"cycles": want, "wall_s": wall, "rtf": rtf(want, w.cpu_hz, wall),
              "reported_cycles": result.get("cycles"), "instructions": result.get("instructions")}
    if w.id == "pico_oled_pwm":
        sample["progress"] = pico_frame_progress(w, result)
    return sample


def measure(cli: Path, w: Workload, repeats: int) -> dict:
    runs = [run_once(cli, w) for _ in range(repeats)]
    rtfs = [rtf(r["cycles"], w.cpu_hz, r["wall_s"]) for r in runs]
    med = statistics.median(rtfs)
    pick = runs[rtfs.index(med)] if med in rtfs else runs[0]
    return {
        "board": w.board,
        "chip": w.chip,
        "workload": w.id,
        "cpu_hz": w.cpu_hz,
        "guest_s": pick["cycles"] / w.cpu_hz,
        "wall_s": round(pick["wall_s"], 4),
        "mcycles_per_s": round(pick["cycles"] / pick["wall_s"] / 1e6, 3),
        "rtf": round(med, 4),
        "rtf_min": round(min(rtfs), 4),
        "rtf_best": round(max(rtfs), 5),
        "repeats": repeats,
        "samples": runs,
    }


def judge(rows: list[dict], spin_rtx: dict[str, float], baselines: dict,
          tolerance: float) -> list[str]:
    """Attach ``index``/``baseline_index`` to rows; return failure messages.

    ``index`` is the best workload RTF over the best spin RTx of the same chip
    in the same run (``spin_rtx``, targeting ~1 s of wall clock). Best-of-N
    reduces contention noise; the ratio cancels only common host scaling.
    Spin-only improvements and different instruction mixes also move it.
    """
    failures = []
    for row in rows:
        spin = spin_rtx.get(row["chip"])
        row["spin_reference_rtx"] = spin
        if not spin:
            row["index"] = None
            continue
        row["index"] = float(f"{row.get('rtf_best', row['rtf']) / spin:.4g}")
        base = (baselines.get(row["workload"]) or {}).get("index")
        row["baseline_index"] = base
        if base is None:
            continue
        floor = base * (1.0 - tolerance)
        if row["index"] < floor:
            drop = 1.0 - row["index"] / base
            failures.append(
                f"{row['workload']} ({row['chip']}): index {row['index']:.4g} is "
                f"{drop:.0%} below baseline {base:.4g} (tolerance {tolerance:.0%})"
            )
    return failures


def git_commit() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, capture_output=True,
            text=True, check=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def report(rows: list[dict], failures: list[str], tolerance: float, *,
           spin_reference: dict | None = None, provenance: dict | None = None) -> dict:
    return {
        "schema": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "commit": os.environ.get("GITHUB_SHA") or git_commit(),
        "runner": os.environ.get("RUNNER_NAME") or platform.node(),
        "host": {"machine": platform.machine(), "processor": platform.processor()},
        "tolerance": tolerance,
        "ok": not failures,
        "failures": failures,
        "boards": rows,
        "spin_reference": spin_reference or {},
        "provenance": provenance or {},
    }


def print_table(rows: list[dict]) -> None:
    print("\nworkload          chip            guest s   wall s   Mcyc/s   x real time   index")
    print("-" * 84)
    for r in rows:
        idx = "-" if r.get("index") is None else f"{r['index']:.4g}"
        print(
            f"{r['workload']:<17} {r['chip']:<15} {r['guest_s']:7.2f} {r['wall_s']:8.2f} "
            f"{r['mcycles_per_s']:8.2f} {r['rtf']:10.3f}x   {idx}"
        )


def run(cli: Path, chips: set[str], spin_rtx: dict[str, float], repeats: int,
        tolerance: float, out: Path | None, update: bool = False, *,
        spin_reference: dict | None = None) -> dict | None:
    selected = [w for w in WORKLOADS if w.chip in chips]
    if not selected:
        return None
    provenance = {}
    rows = []
    measurement_error = None
    try:
        provenance = measurement_provenance(cli, selected)
        for w in selected:
            rows.append(measure(cli, w, repeats))
    except RuntimeError as exc:
        measurement_error = str(exc)
        if not provenance:
            provenance = {"recording_error": measurement_error}
    baselines = json.loads(BASELINES.read_text()) if BASELINES.exists() and measurement_error is None else {}
    failures = [measurement_error] if measurement_error is not None else judge(rows, spin_rtx, baselines, tolerance)
    # A progress/measurement failure must overwrite stale output and must
    # never become a baseline, including when --update-rtf was requested.
    if update and measurement_error is None:
        for r in rows:
            if r.get("index") is not None:
                baselines[r["workload"]] = {"index": r["index"]}
        BASELINES.write_text(json.dumps(baselines, indent=2, sort_keys=True) + "\n")
        failures = []
    print_table(rows)
    doc = report(rows, failures, tolerance, spin_reference=spin_reference, provenance=provenance)
    if out:
        out.write_text(json.dumps(doc, indent=2) + "\n")
    for f in failures:
        print(f"RTF regression: {f}")
    return doc
