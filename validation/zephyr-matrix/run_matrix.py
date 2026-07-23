#!/usr/bin/env python3
"""Zephyr sample × LabWired board matrix.

For every board in boards.yaml and every level (L0/L1/L2):
  1. west build the in-tree sample for the Zephyr board
  2. labwired test against the system manifest
  3. Record pass/fail + UART tail

Shared engine: validation/matrix_lib/  (see docs/engineering/test_harness.md)

Usage (from repo core/):
  python3 validation/zephyr-matrix/run_matrix.py
  python3 validation/zephyr-matrix/run_matrix.py --boards stm32l476,nrf52840
  python3 validation/zephyr-matrix/run_matrix.py --levels L0_hello,L1_sleep
  python3 validation/zephyr-matrix/run_matrix.py --no-build
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

_VALIDATION = Path(__file__).resolve().parent.parent
if str(_VALIDATION) not in sys.path:
    sys.path.insert(0, str(_VALIDATION))

from matrix_lib import find_labwired, render_scoreboard, run_labwired, write_test_script  # noqa: E402
from matrix_lib.invoke import CORE_ROOT  # noqa: E402

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML required — pip install pyyaml", file=sys.stderr)
    sys.exit(2)

MATRIX_DIR = Path(__file__).resolve().parent
DEFAULT_OUT = MATRIX_DIR / "out"


def find_west(zephyrproject: Path) -> Path | None:
    cand = zephyrproject / ".venv" / "bin" / "west"
    if cand.is_file():
        return cand
    w = shutil.which("west")
    return Path(w) if w else None


def load_config() -> dict:
    with open(MATRIX_DIR / "boards.yaml", encoding="utf-8") as f:
        return yaml.safe_load(f)


def west_build(
    west: Path,
    zephyr_base: Path,
    board: str,
    sample_dir: Path,
    build_dir: Path,
    log_path: Path,
    timeout: int,
) -> tuple[bool, str, Path | None]:
    build_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["ZEPHYR_BASE"] = str(zephyr_base)
    env.setdefault("ZEPHYR_TOOLCHAIN_VARIANT", "gnuarmemb")
    env.setdefault("GNUARMEMB_TOOLCHAIN_PATH", "/usr")

    cmd = [
        str(west),
        "build",
        "-p",
        "always",
        "-b",
        board,
        str(sample_dir),
        "-d",
        str(build_dir),
    ]
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            cwd=str(zephyr_base.parent),
        )
    except subprocess.TimeoutExpired as e:
        log_path.write_text(
            (e.stdout or "") + "\nTIMEOUT\n" + (e.stderr or ""), encoding="utf-8"
        )
        return False, "build_timeout", None
    except FileNotFoundError:
        log_path.write_text("west not found\n", encoding="utf-8")
        return False, "toolchain_missing", None

    log_path.write_text(proc.stdout + "\n--- stderr ---\n" + proc.stderr, encoding="utf-8")
    if proc.returncode != 0:
        blob = (proc.stdout + proc.stderr).lower()
        if (
            "toolchain not found" in blob
            or "no toolchain" in blob
            or "unknown board" in blob
            or "invalid board" in blob
            or "board not found" in blob
        ):
            return False, "toolchain_missing", None
        return False, "build_fail", None

    elf = build_dir / "zephyr" / "zephyr.elf"
    if not elf.is_file():
        return False, "elf_missing", None
    return True, "ok", elf


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--boards", help="Comma-separated board ids")
    ap.add_argument("--levels", help="Comma-separated level ids")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--labwired", help="Path to labwired binary")
    ap.add_argument(
        "--zephyrproject",
        type=Path,
        default=Path(os.environ.get("ZEPHYRPROJECT", Path.home() / "zephyrproject")),
    )
    ap.add_argument("--no-build", action="store_true", help="Reuse existing ELFs under out/")
    ap.add_argument("--build-timeout", type=int, default=600)
    ap.add_argument("--sim-timeout", type=int, default=300)
    ap.add_argument("--publish", action="store_true", help="Write docs/coverage/zephyr-scoreboard.md")
    args = ap.parse_args()

    cfg = load_config()
    levels = cfg["levels"]
    boards = cfg["boards"]
    if args.boards:
        want = {x.strip() for x in args.boards.split(",")}
        boards = [b for b in boards if b["id"] in want]
    if args.levels:
        want = {x.strip() for x in args.levels.split(",")}
        levels = [lv for lv in levels if lv["id"] in want]

    labwired = find_labwired(args.labwired)
    zephyr_base = args.zephyrproject / "zephyr"
    west = find_west(args.zephyrproject)

    args.out.mkdir(parents=True, exist_ok=True)
    results: list[dict] = []

    print(f"labwired: {labwired}")
    print(f"zephyr:   {zephyr_base} (exists={zephyr_base.is_dir()})")
    print(f"west:     {west}")
    print(f"cells:    {len(boards)} boards × {len(levels)} levels = {len(boards)*len(levels)}")

    for board in boards:
        for level in levels:
            bid = board["id"]
            lid = level["id"]
            cell = args.out / bid / lid
            cell.mkdir(parents=True, exist_ok=True)
            marker = level["marker"]
            max_steps = int(level.get("max_steps", 2_000_000))
            max_steps += int(board.get("max_steps_bonus", 0))
            if lid != "L0_hello" and board.get("max_steps"):
                max_steps = max(max_steps, int(board["max_steps"]))

            row: dict = {
                "board": bid,
                "level": lid,
                "sketch": lid,  # scoreboard cell key shared with arduino renderer
                "zephyr_board": board["zephyr_board"],
                "marker": marker,
                "status": "skip",
                "uart": "",
            }
            print(f"\n=== {bid} / {lid} ({board['zephyr_board']}) ===")

            elf_pub = cell / "zephyr.elf"
            if not args.no_build:
                if west is None or not zephyr_base.is_dir():
                    row["status"] = "toolchain_missing"
                    results.append(row)
                    print(f"  -> {row['status']}")
                    continue
                sample = MATRIX_DIR / level["sample"]
                build_dir = cell / "build"
                ok, reason, elf = west_build(
                    west,
                    zephyr_base,
                    board["zephyr_board"],
                    sample,
                    build_dir,
                    cell / "build.log",
                    args.build_timeout,
                )
                if not ok or elf is None:
                    row["status"] = reason
                    results.append(row)
                    print(f"  -> {row['status']}")
                    continue
                shutil.copy2(elf, elf_pub)
            else:
                if not elf_pub.is_file():
                    row["status"] = "elf_missing"
                    results.append(row)
                    print(f"  -> {row['status']} (pass --no-build only with prior ELFs)")
                    continue

            system = (MATRIX_DIR / board["system"]).resolve()
            if not system.is_file():
                row["status"] = "boot_fail"
                row["uart"] = f"missing system {system}"
                results.append(row)
                print(f"  -> missing system {system}")
                continue

            script = cell / "test.yaml"
            run_dir = cell / "run"
            write_test_script(script, elf_pub, system, marker, max_steps)
            status, detail = run_labwired(labwired, script, run_dir, args.sim_timeout)
            row["status"] = status
            row["detail"] = detail
            row["uart"] = detail.get("uart_tail", "")
            row["stop_reason"] = (detail.get("result") or {}).get("stop_reason")
            results.append(row)
            uart_show = row["uart"][:80].replace("\n", "\\n")
            print(f"  -> {status}  uart={uart_show!r}")

    (args.out / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")

    scoreboard = render_scoreboard(
        results,
        title="Zephyr × LabWired board matrix",
        generator="validation/zephyr-matrix/run_matrix.py",
        cell_key="level",
        board_ids=[b["id"] for b in boards],
        cell_ids=[lv["id"] for lv in levels],
        legend=(
            "Legend: ✅ pass · 🔧 build fail · 📦 toolchain · 🔴 boot/empty UART · "
            "🟠 marker missing · 🟣 unmodeled/sim · ⏱️ timeout"
        ),
    )
    (args.out / "scoreboard.md").write_text(scoreboard, encoding="utf-8")
    if args.publish:
        pub = CORE_ROOT / "docs" / "coverage" / "zephyr-scoreboard.md"
        pub.parent.mkdir(parents=True, exist_ok=True)
        pub.write_text(scoreboard, encoding="utf-8")

    n_pass = sum(1 for r in results if r["status"] == "pass")
    print(f"\nDone: {n_pass}/{len(results)} pass. Scoreboard: {args.out / 'scoreboard.md'}")
    return 0 if n_pass == len(results) and results else 1


if __name__ == "__main__":
    sys.exit(main())
