#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""LabWired vs Renode wall-time gate: Zephyr L0 hello on nRF52840.

Same pinned ELF, stop when UART contains LW_Z0_OK.

LabWired uses `labwired test` with stop_when_assertions_pass and a 1000-step
settle window (WFI is interpreted after latch — not skipped to RTC overflow).
Renode runs 2 ms of virtual time at 64 MIPS (>> the ~19k boot instructions)
with SetAdvanceImmediately, then the host checks the UART file.

Fail if LabWired process wall time is worse than Renode's by more than --slack
(default 20%).

    python3 scripts/perf/compare_renode_hello.py \\
        --labwired target/release/labwired \\
        --renode /path/to/renode \\
        --output-json renode-hello.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ELF = REPO_ROOT / "tests/fixtures/nrf52840-zephyr-l0-hello.elf"
SYSTEM = REPO_ROOT / "validation/zephyr-matrix/systems/nrf52840.yaml"
MARKER = "LW_Z0_OK"
# arm-none-eabi-strip -g of the Zephyr 3.7.2 L0 hello image for nrf52840dk.
ELF_SHA256 = "a05bad531a5f47bf01f0f86bca28e9a309e20436d60ca99c22220dc670401a32"
RENODE_RUNFOR_S = "0.002"
HEX_RE = re.compile(r"^0x([0-9A-Fa-f]+)\s*$", re.M)
STARTED_RE = re.compile(r"\[(\d+:\d+:\d+\.\d+)\] \[INFO\] nrf52840: Machine started")
PAUSED_RE = re.compile(r"\[(\d+:\d+:\d+\.\d+)\] \[INFO\] nrf52840: Machine paused")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    h.update(path.read_bytes())
    return h.hexdigest()


def verdict(labwired_s: float, renode_s: float, slack: float) -> bool:
    """True when LabWired is not slower than Renode beyond slack."""
    if renode_s <= 0 or labwired_s < 0:
        return False
    return labwired_s <= renode_s * (1.0 + slack)


def _ts(s: str) -> float:
    h, m, rest = s.split(":")
    return int(h) * 3600 + int(m) * 60 + float(rest)


def run_labwired(cli: Path, work: Path) -> dict:
    script = work / "script.yaml"
    out_dir = work / "lw-out"
    out_dir.mkdir()
    script.write_text(
        f"""schema_version: "1.0"
inputs:
  firmware: "{ELF}"
  system: "{SYSTEM}"
limits:
  max_steps: 2000000
  max_uart_bytes: 65536
  stop_when_assertions_pass: true
  stop_when_assertions_pass_settle_steps: 1000
  stop_when_assertions_pass_min_steps: 0
assertions:
  - uart_contains: "{MARKER}"
"""
    )
    env = dict(os.environ)
    env["RUST_LOG"] = "error"
    t0 = time.perf_counter()
    proc = subprocess.run(
        [
            str(cli),
            "test",
            "--script",
            str(script),
            "--output-dir",
            str(out_dir),
            "--no-uart-stdout",
        ],
        capture_output=True,
        text=True,
        env=env,
    )
    wall = time.perf_counter() - t0
    result_path = out_dir / "result.json"
    uart = (out_dir / "uart.log").read_text(errors="replace") if (out_dir / "uart.log").exists() else ""
    result = {}
    if result_path.exists():
        result = json.loads(result_path.read_text())
    ok = proc.returncode == 0 and MARKER in uart and result.get("status") == "pass"
    return {
        "wall_s": wall,
        "ok": ok,
        "rc": proc.returncode,
        "steps": result.get("steps_executed"),
        "cycles": result.get("cycles"),
        "stop_reason": result.get("stop_reason"),
        "uart_ok": MARKER in uart,
        "stderr_tail": (proc.stderr or "")[-1500:],
    }


def run_renode(renode: Path, work: Path) -> dict:
    uart_path = work / "renode-uart.txt"
    resc = work / "hello.resc"
    log_path = work / "renode.log"
    resc.write_text(
        f"""using sysbus
mach create "nrf52840"
machine LoadPlatformDescription @platforms/cpus/nrf52840.repl
sysbus LoadELF @{ELF}
sysbus.cpu PerformanceInMips 64
sysbus.uart0 CreateFileBackend @{uart_path} true
emulation SetGlobalQuantum "0.000100"
emulation SetAdvanceImmediately true
emulation RunFor "{RENODE_RUNFOR_S}"
sysbus.cpu ExecutedInstructions
quit
"""
    )
    t0 = time.perf_counter()
    proc = subprocess.run(
        [str(renode), "--disable-gui", "--console", str(resc)],
        capture_output=True,
        text=True,
        cwd=str(renode.parent),
    )
    wall = time.perf_counter() - t0
    out = (proc.stdout or "") + "\n" + (proc.stderr or "")
    log_path.write_text(out)
    uart = uart_path.read_text(errors="replace") if uart_path.exists() else ""
    instr = None
    hx = HEX_RE.findall(out)
    if hx:
        instr = int(hx[-1], 16)
    exec_s = None
    sm, pm = STARTED_RE.search(out), PAUSED_RE.findall(out)
    if sm and pm:
        exec_s = _ts(pm[-1]) - _ts(sm.group(1))
        if exec_s < 0:
            exec_s += 24 * 3600
    return {
        "wall_s": wall,
        "exec_s": exec_s,
        "ok": MARKER in uart,
        "rc": proc.returncode,
        "instructions": instr,
        "uart_ok": MARKER in uart,
        "runfor_s": float(RENODE_RUNFOR_S),
        "stderr_tail": out[-1500:],
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--labwired", required=True, type=Path)
    p.add_argument("--renode", required=True, type=Path)
    p.add_argument("--slack", type=float, default=0.20)
    p.add_argument("--output-json", type=Path, default=Path("renode-hello.json"))
    p.add_argument("--skip-sha", action="store_true", help="do not pin ELF sha256 (local debug only)")
    args = p.parse_args()

    if not ELF.is_file():
        print(f"missing ELF {ELF}", file=sys.stderr)
        return 2
    if not SYSTEM.is_file():
        print(f"missing system {SYSTEM}", file=sys.stderr)
        return 2
    digest = sha256_file(ELF)
    if not args.skip_sha and digest != ELF_SHA256:
        print(
            f"ELF sha256 mismatch: got {digest} want {ELF_SHA256}",
            file=sys.stderr,
        )
        return 2
    if not args.labwired.is_file():
        print(f"missing labwired {args.labwired}", file=sys.stderr)
        return 2
    if not args.renode.is_file():
        print(f"missing renode {args.renode}", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="lw-renode-hello-") as tmp:
        work = Path(tmp)
        lw = run_labwired(args.labwired, work)
        rd = run_renode(args.renode, work)

    passed = bool(lw["ok"] and rd["ok"] and verdict(lw["wall_s"], rd["wall_s"], args.slack))
    ratio = lw["wall_s"] / rd["wall_s"] if rd["wall_s"] else None
    summary = {
        "workload": "zephyr-l0-hello-nrf52840",
        "marker": MARKER,
        "elf": str(ELF),
        "elf_sha256": digest,
        "slack": args.slack,
        "labwired": {k: v for k, v in lw.items() if k != "stderr_tail"},
        "renode": {k: v for k, v in rd.items() if k != "stderr_tail"},
        "ratio_labwired_over_renode": ratio,
        "pass": passed,
    }
    args.output_json.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    if not lw["ok"]:
        print("LabWired failed to print the marker:\n" + lw["stderr_tail"], file=sys.stderr)
    if not rd["ok"]:
        print("Renode failed to print the marker:\n" + rd["stderr_tail"], file=sys.stderr)
    if lw["ok"] and rd["ok"] and not passed:
        print(
            f"LabWired {lw['wall_s']:.3f}s is slower than Renode {rd['wall_s']:.3f}s "
            f"(slack {args.slack:.0%})",
            file=sys.stderr,
        )
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
