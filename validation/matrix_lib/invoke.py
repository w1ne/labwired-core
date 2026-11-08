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
    wall_time_ms: int | None = None,
) -> None:
    """Write a schema 1.0 labwired test script (UART marker oracle).

    `wall_time_ms` bounds the run by host time. `labwired test` refuses a
    `max_steps` above 50M unless one is set (the guard catches CI
    misconfiguration), so a board whose stock core legitimately spends more
    than that before `setup()` declares it.
    """
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
            **({"wall_time_ms": wall_time_ms} if wall_time_ms else {}),
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


def _count_rmt_tx(result: dict[str, Any]) -> int:
    """Sum RMT TX_START counts from inspect artifacts (C3/S3 rgbLedWrite)."""
    inspect = result.get("inspect") or {}
    total = 0
    for peri in inspect.get("peripherals") or []:
        for art in peri.get("artifacts") or []:
            if art.get("kind") == "rmt_tx":
                meta = art.get("meta") or {}
                try:
                    total += int(meta.get("count") or 0)
                except (TypeError, ValueError):
                    pass
    return total


def classify_failure(
    proc: subprocess.CompletedProcess[str],
    result: dict[str, Any],
    uart: str,
) -> str:
    """Map labwired exit + result.json to a matrix status string.

    ``unmodeled`` means the model hit a coverage gap: unmapped MMIO or an
    undecoded instruction. The engine records those STRUCTURALLY in
    ``result["fidelity"]`` (core's ``FidelityReport``, omitted when empty), so
    that list — not a word search — is what decides the label.

    ⚠️ The text heuristics below read ONLY stdout/stderr. They must never be
    run over ``json.dumps(result)``: the result carries fields whose VALUES are
    words like "unsupported" regardless of how the run went. ``memory`` is one
    — on Xtensa it always reports ``main_stack_method: "unsupported"`` because
    stack-usage measurement is not implemented for that arch. Folding the
    result into the blob made EVERY failing Xtensa cell report ``unmodeled``,
    so a dead boot and a genuine coverage gap were indistinguishable, and a
    wedge that burned its whole step budget was filed as a missing peripheral.
    """
    if proc.returncode == 0:
        return "pass"

    status = str(result.get("status", "")).lower()
    stop = str(result.get("stop_reason", "")).lower()
    # Structured, authoritative: the engine's own coverage-gap list.
    # Approximations (derived device time, and anything else that is not an
    # unmapped access or an undecoded instruction) are honest notes. They are
    # present on passing runs too; treating them as gaps labelled every STM32
    # oracle miss `unmodeled`.
    fidelity = result.get("fidelity") or []
    if isinstance(fidelity, list):
        gap_kinds = {"unmapped_mmio", "undecoded_instruction"}
        if any(isinstance(g, dict) and g.get("kind") in gap_kinds for g in fidelity):
            return "unmodeled"
    elif fidelity:
        return "unmodeled"

    # Structured stop reasons that ARE faults.
    if stop in ("memory_violation", "exception", "fault"):
        return "unmodeled"

    # Text fallback for engine builds/paths that report a gap only in prose.
    # stdout+stderr only — see the warning above.
    blob = (proc.stdout or "") + (proc.stderr or "")
    blob = blob.lower()
    gap_phrases = (
        "unmodeled",
        "unmodelled",
        "unimplemented",
        "unknown instruction",
        "unknown 32-bit instruction",
        "bus read fault",
        "bus write fault",
        "outside of memory map",
        "memory access violation",
        "memory_violation",
        "not modeled",
        "unsupported instruction",
        "unsupported opcode",
        "unsupported peripheral",
    )
    if any(s in blob for s in gap_phrases):
        return "unmodeled"
    if status in ("error", "runtime_error") and stop not in (
        "max_steps",
        "assertions_passed",
        "assertions_failed",
    ):
        return "sim_error"

    # No gap, no fault: the run executed and simply never got where it should.
    # `max_steps` with nothing on the console is a WEDGE — the firmware is
    # spinning, not missing a peripheral. It is a boot failure, and saying so
    # is what makes it findable.
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
    min_rmt_tx: int | None = None,
    extra_env: dict[str, str] | None = None,
) -> tuple[str, dict[str, Any]]:
    """Run `labwired test`. Returns (status, detail).

    If ``min_logic_edges`` is set and the UART oracle passes, require at least
    that many logic transitions across watched channels (L2 GPIO honesty).
    If ``min_rmt_tx`` is set, require at least that many RMT TX_START pulses
    (inspect artifact ``rmt_tx``) — C3/S3 RGB ``rgbLedWrite`` path.
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
    if status == "pass" and min_rmt_tx is not None and min_rmt_tx > 0:
        n = _count_rmt_tx(result)
        detail["rmt_tx_count"] = n
        if n < min_rmt_tx:
            status = "oracle_fail"
            detail["oracle"] = f"rmt TX_START count {n} < required min_rmt_tx {min_rmt_tx}"
    return status, detail
