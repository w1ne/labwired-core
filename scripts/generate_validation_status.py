#!/usr/bin/env python3
"""
Generate docs/boards/VALIDATION_STATUS.md from validation/manifest.yaml and
enforce the no-silent-decay rule.

The board docs drift; this generated table does not. It is the single
machine-checked view of what is actually validated, against which silicon, and
on what date — plus an automated DRIFT gate that catches the case where a
peripheral model changed AFTER the board's last silicon capture.

Modes
-----
  (default)      regenerate docs/boards/VALIDATION_STATUS.md in place
  --check        regenerate to memory and diff against the committed file;
                 exit 1 if they differ (run this in CI so the doc cannot go stale)
  --drift        exit 1 if any silicon-tier board has DRIFTED past its drift_ack
  (you normally run CI with BOTH:  --check --drift)

Either gate flag also runs the drift-watch COVERAGE audit (see below).

Drift
-----
For each board with `silicon.last_capture`, the newest git commit date across
`models` is compared to the capture date. If newer, the board has drifted. A
dated `drift_ack` (>= the newest model date) is an explicit human acknowledgement
that keeps it green; any later model change re-breaks the gate.

Drift-watch coverage
--------------------
The drift gate can only see what `models` lists, and an incomplete list fails
OPEN: the board reads "fresh" forever while the files its claim rests on change
underneath. esp32c3 shipped that way — its tier is a reset-state oracle asserted
against the declarative descriptors in `configs/peripherals/esp32c3/`, and all
29 of them were outside its watch list, as was the shared `esp_uart.rs` its real
UART0/UART1 register map moved into on 2026-07-28.

So we audit the watch list itself, mechanically:
  * every `path:` a board's chip yaml wires (resolved relative to the chip yaml)
    must be covered by an entry in that board's `models`; and
  * every listed `models` path must exist — a stale path is a silently disabled
    watch, not a warning.
Coded (non-declarative) peripheral impls cannot be derived from the yaml and are
still listed by hand; this audit closes the mechanical half of the hole.

Needs PyYAML (pip install pyyaml) and a full-history checkout (fetch-depth: 0)
so `git log -- <path>` resolves dates.
"""

from __future__ import annotations

import argparse
import difflib
import posixpath
import re
import subprocess
import sys
from datetime import date, datetime
from pathlib import Path

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML required — `pip install pyyaml`", file=sys.stderr)
    sys.exit(2)

CORE_ROOT = Path(__file__).resolve().parent.parent
MANIFEST = CORE_ROOT / "validation" / "manifest.yaml"
OUT_DOC = CORE_ROOT / "docs" / "boards" / "VALIDATION_STATUS.md"

TIER_BADGE = {
    "silicon-verified": "🟢 silicon-verified",
    "silicon-smoke": "🟢 silicon-smoke",
    "sim-validated": "🔵 sim-validated (deep model, no HW diff)",
    "smoke-manual": "🟡 smoke-manual",
    "structural": "⚪ structural",
}


def parse_iso(v: str) -> datetime:
    """datetime.fromisoformat, but accepting the trailing 'Z' git emits for UTC.

    `git log --format=%cI` renders UTC as '...T00:11:20Z'. Only Python 3.11+
    accepts that suffix, so on the macOS system interpreter (3.9) every local
    run of this script died in newest_commit_date() while CI's 3.12 passed —
    the regeneration command the error message itself tells you to run was
    impossible to run on a stock Mac. Normalise instead of requiring 3.11.
    """
    return datetime.fromisoformat(v[:-1] + "+00:00" if v.endswith("Z") else v)


# `path: "../peripherals/esp32c3/system.yaml"` inside a chip yaml.
CHIP_YAML_PATH_RE = re.compile(r"""^\s*path:\s*["']?([^"'\s#]+)["']?\s*(?:#.*)?$""", re.M)


def covers(model_entry: str, path: str) -> bool:
    """True if a `models` entry (file or directory) covers `path`."""
    return path == model_entry or path.startswith(model_entry.rstrip("/") + "/")


def watch_gaps(board: dict) -> tuple[list[str], list[str]]:
    """(uncovered chip-yaml paths, listed model paths that do not exist).

    Both are drift-gate holes that fail OPEN — the board keeps reading "fresh"
    while something its claim depends on is unwatched. See module docs.
    """
    models = board.get("models", [])
    missing = [m for m in models if not (CORE_ROOT / m).exists()]

    chip_rel = board.get("chip")
    uncovered: list[str] = []
    if chip_rel and (CORE_ROOT / chip_rel).exists():
        chip_dir = Path(chip_rel).parent
        for raw in CHIP_YAML_PATH_RE.findall((CORE_ROOT / chip_rel).read_text()):
            # Chip yamls reach configs/peripherals via `../`; normalise textually
            # (posixpath.normpath, not resolve()) so the result is repo-relative
            # and stable regardless of where the checkout lives or symlinks.
            wired = posixpath.normpath(posixpath.join(chip_dir.as_posix(), raw))
            if not any(covers(m, wired) for m in models):
                uncovered.append(wired)
    return sorted(set(uncovered)), missing


def audit_watch_lists(manifest: dict) -> int:
    """Fail the build on any drift-watch hole. Returns 0 or 1."""
    rc = 0
    for b in manifest["boards"]:
        uncovered, missing = watch_gaps(b)
        if missing:
            print(
                f"ERROR: {b['id']}: `models` lists path(s) that do not exist — a stale "
                f"entry watches nothing:\n  " + "\n  ".join(missing),
                file=sys.stderr,
            )
            rc = 1
        if uncovered:
            print(
                f"ERROR: {b['id']}: {len(uncovered)} path(s) wired by {b['chip']} are NOT "
                "covered by its `models` drift-watch list, so a change to them cannot "
                "fail the drift gate:\n  " + "\n  ".join(uncovered) + "\n"
                "       Add them (a parent directory counts) to validation/manifest.yaml.",
                file=sys.stderr,
            )
            rc = 1
    return rc


def newest_commit_date(paths: list[str]) -> date | None:
    """Newest committer date (YYYY-MM-DD) across the given repo paths, or None."""
    newest: date | None = None
    for rel in paths:
        target = CORE_ROOT / rel
        if not target.exists():
            # A listed model path that no longer exists is itself a manifest bug;
            # audit_watch_lists() turns this into a hard failure under the gates.
            print(f"WARNING: manifest model path does not exist: {rel}", file=sys.stderr)
            continue
        out = subprocess.run(
            ["git", "log", "-1", "--format=%cI", "--", rel],
            cwd=CORE_ROOT,
            capture_output=True,
            text=True,
        )
        iso = out.stdout.strip()
        if not iso:
            continue
        d = parse_iso(iso).date()
        if newest is None or d > newest:
            newest = d
    return newest


def as_date(v) -> date | None:
    if v is None:
        return None
    if isinstance(v, date):
        return v
    return parse_iso(str(v)).date()


def evaluate(board: dict) -> dict:
    """Compute drift status for one board."""
    silicon = board.get("silicon")
    models = board.get("models", [])
    newest = newest_commit_date(models)
    capture = as_date(silicon["last_capture"]) if silicon else None
    ack = as_date(board.get("drift_ack"))

    drifted = bool(capture and newest and newest > capture)
    acked = bool(ack and newest and ack >= newest)
    # A board with no silicon capture cannot "drift" — it never claimed silicon.
    failing = drifted and not acked

    if not silicon:
        status = "no silicon capture"
    elif not drifted:
        status = "✅ fresh"
    elif acked:
        status = f"⚠ drift acked {ack:%Y-%m-%d} (re-capture pending)"
    else:
        status = f"✖ DRIFT — model {newest:%Y-%m-%d} > capture; RE-CAPTURE"

    return {
        "newest_model": newest,
        "capture": capture,
        "drifted": drifted,
        "failing": failing,
        "status": status,
    }


def render(manifest: dict) -> str:
    boards = manifest["boards"]
    lines: list[str] = []
    lines.append("<!-- GENERATED by scripts/generate_validation_status.py — DO NOT EDIT BY HAND.")
    lines.append("     Source of truth: validation/manifest.yaml. Regenerated and gated on every CI run. -->")
    lines.append("")
    lines.append("# Board validation status")
    lines.append("")
    lines.append(
        "Machine-generated from `validation/manifest.yaml`. CI regenerates this on "
        "every run (`--check`) and fails if a peripheral model changed after a "
        "board's last silicon capture without a dated `drift_ack` (`--drift`). "
        "Tiers: 🟢 silicon · 🟡 manual-smoke · ⚪ structural."
    )
    lines.append("")
    lines.append("| Board | Tier | Last silicon capture | Newest model | Status |")
    lines.append("|-------|------|----------------------|--------------|--------|")
    for b in boards:
        ev = evaluate(b)
        tier = TIER_BADGE.get(b["tier"], b["tier"])
        cap = f"{ev['capture']:%Y-%m-%d}" if ev["capture"] else "—"
        nm = f"{ev['newest_model']:%Y-%m-%d}" if ev["newest_model"] else "—"
        lines.append(f"| `{b['id']}` | {tier} | {cap} | {nm} | {ev['status']} |")
    lines.append("")

    # Per-board detail
    for b in boards:
        ev = evaluate(b)
        lines.append(f"## `{b['id']}` — {TIER_BADGE.get(b['tier'], b['tier'])}")
        lines.append("")
        lines.append(f"- Doc: [`{b['doc']}`]({Path(b['doc']).name})  ·  Chip: `{b['chip']}`")
        if b.get("note"):
            lines.append(f"- Note: {b['note']}")
        sil = b.get("silicon")
        if sil:
            lines.append(
                f"- Silicon: **{ev['capture']:%Y-%m-%d}** on {sil.get('probe', '?')} — {sil.get('result', '')}"
            )
        else:
            lines.append("- Silicon: none — not validated against real hardware.")
        for t in b.get("offline_tests", []):
            lines.append(f"  - offline (CI): {t}")
        lines.append(f"- Drift status: **{ev['status']}**")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="fail if committed doc is stale")
    ap.add_argument("--drift", action="store_true", help="fail if any board drifted past its ack")
    args = ap.parse_args()

    manifest = yaml.safe_load(MANIFEST.read_text())
    rendered = render(manifest)

    rc = 0

    # Coverage before content: a stale doc is visible, an unwatched model is not.
    if args.check or args.drift:
        rc |= audit_watch_lists(manifest)

    if args.check:
        existing = OUT_DOC.read_text() if OUT_DOC.exists() else ""
        if existing != rendered:
            # Print the actual difference. This doc is rendered partly from git
            # commit dates, so a checkout whose history differs from yours can
            # fail this gate while it passes locally — and "is out of date" on
            # its own gives a CI reader nothing to act on.
            diff = "".join(
                difflib.unified_diff(
                    existing.splitlines(keepends=True),
                    rendered.splitlines(keepends=True),
                    fromfile=f"{OUT_DOC.name} (committed)",
                    tofile=f"{OUT_DOC.name} (regenerated here)",
                )
            )
            print(
                f"ERROR: {OUT_DOC.relative_to(CORE_ROOT)} is out of date.\n"
                "       Run: python3 scripts/generate_validation_status.py\n"
                f"{diff}",
                file=sys.stderr,
            )
            rc = 1
    elif not args.drift:
        # Pure generate mode (no gate flags): rewrite the doc in place.
        OUT_DOC.write_text(rendered)
        print(f"wrote {OUT_DOC.relative_to(CORE_ROOT)}")

    if args.drift:
        failing = [b["id"] for b in manifest["boards"] if evaluate(b)["failing"]]
        if failing:
            print(
                "ERROR: silicon validation has DRIFTED (model changed after last capture, "
                "no covering drift_ack):\n  " + "\n  ".join(failing) + "\n"
                "       Re-run the live diff and bump silicon.last_capture, or set a dated "
                "drift_ack in validation/manifest.yaml.",
                file=sys.stderr,
            )
            rc = 1

    return rc


if __name__ == "__main__":
    sys.exit(main())
