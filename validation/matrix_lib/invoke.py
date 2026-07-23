"""labwired binary discovery, test script write, run + classify."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError as e:  # pragma: no cover
    raise SystemExit("ERROR: PyYAML required — pip install pyyaml") from e

# validation/matrix_lib/ → validation/ → core/
CORE_ROOT = Path(__file__).resolve().parent.parent.parent


def find_labwired(explicit: str | None = None) -> Path:
    if explicit:
        p = Path(explicit)
        if not p.is_file():
            raise SystemExit(f"labwired binary not found: {p}")
        return p
    for c in (
        CORE_ROOT / "target" / "release" / "labwired",
        CORE_ROOT / "target" / "debug" / "labwired",
        Path(shutil.which("labwired") or ""),
    ):
        if c and c.is_file():
            return c
    raise SystemExit(
        "labwired CLI not found. Build with:\n"
        "  cargo build -p labwired-cli --release\n"
        "or pass --labwired /path/to/labwired"
    )


def write_test_script(
    path: Path,
    firmware: Path,
    system: Path,
    marker: str,
    max_steps: int,
) -> None:
    """Write a schema 1.0 labwired test script (UART marker oracle)."""
    doc = {
        "schema_version": "1.0",
        "inputs": {
            "firmware": str(firmware.resolve()),
            "system": str(system.resolve()),
        },
        "limits": {
            "max_steps": max_steps,
            "max_uart_bytes": 65536,
            "stop_when_assertions_pass": True,
            "stop_when_assertions_pass_settle_steps": 1000,
            "stop_when_assertions_pass_min_steps": 0,
        },
        "assertions": [
            {"uart_contains": marker},
        ],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(yaml.safe_dump(doc, sort_keys=False), encoding="utf-8")


def _count_logic_edges(result: dict[str, Any]) -> int:
    edges = result.get("logic_edges") or {}
    channels = edges.get("channels") or []
    n = 0
    for ch in channels:
        n += len(ch.get("transitions") or [])
    return n


def classify_failure(
    proc: subprocess.CompletedProcess[str],
    result: dict[str, Any],
    uart: str,
) -> str:
    """Map labwired exit + result.json to a matrix status string.

    Prefer structured result fields; fall back to stderr heuristics for
    unmodeled gaps (legacy).
    """
    if proc.returncode == 0:
        return "pass"

    status = str(result.get("status", "")).lower()
    stop = str(result.get("stop_reason", "")).lower()
    blob = (proc.stdout + (proc.stderr or "") + json.dumps(result)).lower()

    # Structured first
    if stop in ("memory_violation", "exception", "fault"):
        return "unmodeled"
    if status in ("error", "runtime_error") and stop not in (
        "max_steps",
        "assertions_passed",
        "assertions_failed",
    ):
        if any(
            s in blob
            for s in (
                "unmodeled",
                "unimplemented",
                "unknown instruction",
                "bus read fault",
                "bus write fault",
                "outside of memory map",
                "memory access violation",
            )
        ):
            return "unmodeled"

    if any(
        s in blob
        for s in (
            "unmodeled",
            "unimplemented",
            "unknown instruction",
            "unknown 32-bit instruction",
            "bus read fault",
            "bus write fault",
            "outside of memory map",
            "memory access violation",
            "memory_violation",
        )
    ) or stop in ("memory_violation", "exception"):
        return "unmodeled"
    if "unsupported" in blob or "not modeled" in blob:
        return "unmodeled"
    if uart.strip() == "":
        return "boot_fail"
    return "oracle_fail"


def run_labwired(
    labwired: Path,
    script: Path,
    out_dir: Path,
    timeout: int,
    *,
    watch_gpio: list[str] | None = None,
    min_logic_edges: int | None = None,
    extra_env: dict[str, str] | None = None,
) -> tuple[str, dict[str, Any]]:
    """Run `labwired test`. Returns (status, detail).

    If ``min_logic_edges`` is set and the UART oracle passes, require at least
    that many logic transitions across watched channels (L2 GPIO honesty).
    """
    out_dir.mkdir(parents=True, exist_ok=True)
    cmd = [
        str(labwired),
        "test",
        "--script",
        str(script),
        "--output-dir",
        str(out_dir),
        "--no-uart-stdout",
    ]
    for spec in watch_gpio or []:
        cmd.extend(["--watch-gpio", spec])

    env = os.environ.copy()
    if extra_env:
        env.update(extra_env)
    # RP2040 Arduino needs mask ROM at 0; bare-metal tests may opt out via empty.
    bootrom = CORE_ROOT / "crates" / "core" / "roms" / "rp2040" / "bootrom.bin"
    if bootrom.is_file():
        env.setdefault("LABWIRED_RP2040_BOOTROM", str(bootrom))

    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=str(CORE_ROOT),
            env=env,
        )
    except subprocess.TimeoutExpired:
        return "timeout", {"stderr": "labwired test timed out"}

    (out_dir / "labwired.stdout").write_text(proc.stdout or "", encoding="utf-8")
    (out_dir / "labwired.stderr").write_text(proc.stderr or "", encoding="utf-8")

    result: dict[str, Any] = {}
    result_path = out_dir / "result.json"
    if result_path.is_file():
        try:
            result = json.loads(result_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            result = {}

    uart_path = out_dir / "uart.log"
    uart = uart_path.read_text(encoding="utf-8", errors="replace") if uart_path.is_file() else ""
    detail: dict[str, Any] = {
        "result": result,
        "uart_tail": uart[-500:],
        "stderr": (proc.stderr or "")[-1500:],
    }

    status = classify_failure(proc, result, uart)
    if status == "pass" and min_logic_edges is not None and min_logic_edges > 0:
        n = _count_logic_edges(result)
        detail["logic_edge_count"] = n
        if n < min_logic_edges:
            status = "oracle_fail"
            detail["oracle"] = (
                f"logic edges {n} < required min_logic_edges {min_logic_edges} "
                f"(watch_gpio={watch_gpio})"
            )
    return status, detail
