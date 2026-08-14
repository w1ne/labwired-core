#!/usr/bin/env python3
"""Every chip we ship must be executed by a CLI gate, or say why not.

The perf gate already answers "is this chip covered" for THROUGHPUT, and it
answers it from disk (`configs/chips/`) so a new descriptor cannot slip past.
Nothing answered the same question for BEHAVIOUR. The Arduino matrix names its
16 boards inline in the workflow, the coverage matrix names its cells inline,
and both lists are hand-written — so a chip added to `configs/chips/` ran real
firmware in no gate at all and nothing said so. 8 of 26 were in that state when
this landed.

Worse, membership in `validation/arduino-matrix/boards.yaml` is not by itself
coverage: a board there that the workflow's matrix does not name runs nowhere.
That is the same shape as the h735 lab whose `io-smoke.yaml` sat in the tree
with no workflow running it (see core-coverage-matrix-smoke.yml).

So coverage here is DERIVED, never declared:

  * a chip is proven by the Arduino matrix only if some board in boards.yaml
    names it AND that board id appears in the workflow's matrix list;
  * a chip is proven by a script only if some workflow (or a board manifest's
    `ci/test.sh`) passes that script to `labwired test`, and the script's
    system resolves to that chip.

`configs/ci/chip-coverage.yaml` is then only for what is NOT proven, with a
reason per chip and a ceiling that can only be lowered. A chip that becomes
proven must leave the file, so the ledger cannot rot into a list of excuses —
and when the last entry goes, so does the file. Its absence is not a hole: with
no entries, every uncovered chip is an error, which is the whole gate.

    python3 scripts/ci/chip_coverage.py            # gate (exit 1 on drift)
    python3 scripts/ci/chip_coverage.py --report   # show who proves what
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import yaml

# Mutable so the tests can point the whole gate at a synthetic tree (see
# --repo-root). Everything below reads it through the helpers, never at import
# time, or a test would silently grade the real repo instead of its fixture.
REPO_ROOT = Path(__file__).resolve().parents[2]


def set_root(root: Path) -> None:
    global REPO_ROOT  # noqa: PLW0603 - one knob, set once per process
    REPO_ROOT = Path(root).resolve()


def chip_dir() -> Path:
    return REPO_ROOT / "configs/chips"


def workflow_dir() -> Path:
    return REPO_ROOT / ".github/workflows"


def arduino_boards() -> Path:
    return REPO_ROOT / "validation/arduino-matrix/boards.yaml"


def arduino_workflow() -> Path:
    return workflow_dir() / "core-arduino-matrix-smoke.yml"


def board_manifest() -> Path:
    return REPO_ROOT / "configs/ci/boards.yml"


def system_dir() -> Path:
    return REPO_ROOT / "configs/systems"


def survival_test() -> Path:
    return REPO_ROOT / "crates/core/tests/firmware_survival.rs"


def ledger_path() -> Path:
    return REPO_ROOT / "configs/ci/chip-coverage.yaml"


# `ci-fixture-*` descriptors are harness plumbing, not silicon we offer. Same
# exclusion the built-in registry test and the perf gate use.
CHIP_EXCLUDE_PREFIX = "ci-fixture-"

# `--script <path>` as written in a workflow step or a board's ci/test.sh, and
# `script: <path>` as written in a workflow matrix include.
SCRIPT_FLAG_RE = re.compile(r"--script\s+[\"']?([\w./-]+\.yaml)")
SCRIPT_KEY_RE = re.compile(r"^\s*script:\s*[\"']?([\w./-]+\.yaml)", re.MULTILINE)
# `system: "<name>"` rows in the firmware-survival table.
SURVIVAL_SYSTEM_RE = re.compile(r"^\s*system:\s*\"([\w.-]+)\"", re.MULTILINE)

KINDS = ("pending", "waived")


class CoverageError(RuntimeError):
    """The ledger and the gates disagree about what is proven."""


def load_yaml(path: Path) -> dict:
    return yaml.safe_load(path.read_text()) or {}


def shipped_chips() -> list[str]:
    """Chip descriptors we ship, FROM DISK — never a hand-kept list."""
    return sorted(
        p.stem
        for p in chip_dir().glob("*.yaml")
        if not p.stem.startswith(CHIP_EXCLUDE_PREFIX)
    )


def chip_of_system(system_path: Path) -> str | None:
    """Resolve a system manifest to the chip stem it runs.

    `chip:` is either a path relative to the manifest or a built-in name; both
    forms end at a `configs/chips/<stem>.yaml`, so both reduce to the stem.
    """
    if not system_path.is_file():
        return None
    try:
        chip = load_yaml(system_path).get("chip")
    except yaml.YAMLError:
        return None
    if not isinstance(chip, str) or not chip:
        return None
    stem = Path(chip).stem if chip.endswith(".yaml") else chip
    return stem or None


def chip_of_script(script_path: Path) -> str | None:
    """Resolve a test script to the chip it exercises, via its system."""
    if not script_path.is_file():
        return None
    try:
        script = load_yaml(script_path)
    except yaml.YAMLError:
        return None
    system = (script.get("inputs") or {}).get("system")
    if not isinstance(system, str) or not system:
        return None
    return chip_of_system((script_path.parent / system).resolve())


def _texts_that_drive_the_cli() -> list[tuple[str, str]]:
    """(source label, text) for every file that can invoke `labwired test`.

    Workflows, plus the `ci/test.sh` of each board in the CI manifest — a board
    entry runs its own script, so a chip proven only that way is still proven.
    """
    out = [(f".github/workflows/{p.name}", p.read_text()) for p in sorted(workflow_dir().glob("*.yml"))]
    if board_manifest().is_file():
        for entry in load_yaml(board_manifest()).get("boards") or []:
            test_sh = REPO_ROOT / str(entry.get("path", "")) / "ci/test.sh"
            if test_sh.is_file():
                out.append((f"{entry.get('id')} (ci/test.sh)", test_sh.read_text()))
    return out


def script_proofs() -> dict[str, set[str]]:
    """chip -> the CI sources that run a real script against it."""
    proofs: dict[str, set[str]] = {}
    for label, text in _texts_that_drive_the_cli():
        paths = set(SCRIPT_FLAG_RE.findall(text)) | set(SCRIPT_KEY_RE.findall(text))
        for rel in paths:
            # Board `ci/test.sh` scripts are written relative to the repo root
            # by convention; both forms resolve against it here.
            chip = chip_of_script((REPO_ROOT / rel).resolve())
            if chip:
                proofs.setdefault(chip, set()).add(f"{label}: {rel}")
    return proofs


def arduino_proofs() -> tuple[dict[str, set[str]], list[str]]:
    """chip -> arduino matrix cells, plus boards.yaml entries that run nowhere.

    A boards.yaml entry the workflow does not name is NOT coverage. Reporting
    it as coverage is exactly how a lab ends up in the tree gating nothing.
    """
    if not arduino_boards().is_file() or not arduino_workflow().is_file():
        raise CoverageError("the Arduino matrix manifest or workflow is missing")
    workflow_text = arduino_workflow().read_text()
    # The matrix lists board ids as `- <id>` under `board:`; match on the exact
    # list item so a substring (esp32 vs esp32s3) cannot forge a hit.
    listed = set(re.findall(r"^\s+-\s+([\w.-]+)\s*$", workflow_text, re.MULTILINE))

    proofs: dict[str, set[str]] = {}
    unrun: list[str] = []
    for board in load_yaml(arduino_boards()).get("boards") or []:
        bid, chip = board.get("id"), board.get("chip")
        if not bid or not chip:
            raise CoverageError(f"arduino board entry {bid!r} has no id/chip")
        if bid in listed:
            proofs.setdefault(chip, set()).add(f"arduino-matrix: {bid}")
        else:
            unrun.append(bid)
    return proofs, unrun


def harness_proofs() -> dict[str, set[str]]:
    """chip -> firmware-survival rows that boot a real ELF on it.

    NOT counted as CLI coverage, on purpose: `firmware_survival.rs` drives the
    engine in-process, so a chip it covers can still be unreachable through the
    binary a user installs. It is reported so the ledger's reasons can say what
    DOES execute the chip today instead of reading as "nothing".
    """
    if not survival_test().is_file():
        return {}
    proofs: dict[str, set[str]] = {}
    for system in set(SURVIVAL_SYSTEM_RE.findall(survival_test().read_text())):
        chip = chip_of_system(system_dir() / f"{system}.yaml")
        if chip:
            proofs.setdefault(chip, set()).add(f"firmware_survival: {system}")
    return proofs


def load_ledger() -> tuple[dict[str, dict], int]:
    """(entries, ceiling). No file at all means no declared gaps.

    That is the end state this gate was built to reach, and it fails CLOSED:
    with zero entries, `evaluate` calls every uncovered chip an error, so
    deleting the file to escape the gate makes it stricter, not quieter. An
    empty file would say the same thing while leaving a page of instructions
    for a list that has nothing in it, so the file goes when the last gap does
    — see the committed-tree test in test_chip_coverage.py.
    """
    if not ledger_path().is_file():
        return {}, 0
    data = load_yaml(ledger_path())
    entries = data.get("uncovered") or {}
    ceiling = data.get("max_uncovered")
    if not isinstance(ceiling, int):
        raise CoverageError("chip-coverage.yaml needs an integer `max_uncovered:` ceiling")
    return entries, ceiling


def evaluate() -> tuple[dict[str, set[str]], list[str], list[str]]:
    """Return (proofs, uncovered chips, errors)."""
    chips = shipped_chips()
    proofs, unrun_boards = arduino_proofs()
    for chip, sources in script_proofs().items():
        proofs.setdefault(chip, set()).update(sources)

    errors: list[str] = []
    for bid in sorted(unrun_boards):
        errors.append(
            f"arduino board '{bid}' is declared in boards.yaml but the matrix in "
            "core-arduino-matrix-smoke.yml does not name it, so nothing runs it"
        )
    for chip in sorted(proofs):
        # `ci-fixture-*` descriptors are gated deliberately and excluded from the
        # shipped list; a proof naming one is expected, not drift.
        if chip not in chips and not chip.startswith(CHIP_EXCLUDE_PREFIX):
            errors.append(
                f"a CI gate runs chip '{chip}', which is not in configs/chips/ — "
                "stale system manifest or renamed descriptor"
            )

    uncovered = [c for c in chips if c not in proofs]
    entries, ceiling = load_ledger()

    for chip in uncovered:
        entry = entries.get(chip)
        if entry is None:
            errors.append(
                f"chip '{chip}' ships but NO CI gate executes firmware on it. Add a cell "
                "(validation/arduino-matrix/boards.yaml + the workflow matrix, or a script "
                "a workflow runs), or add it to configs/ci/chip-coverage.yaml with a reason."
            )
            continue
        if entry.get("kind") not in KINDS:
            errors.append(f"chip '{chip}': ledger `kind` must be one of {KINDS}")
        if not str(entry.get("reason") or "").strip():
            errors.append(f"chip '{chip}': ledger entry needs a concrete `reason`")

    for chip in sorted(entries):
        if chip not in chips:
            errors.append(f"ledger lists '{chip}', which is not a shipped chip — drop the stale entry")
        elif chip in proofs:
            errors.append(
                f"chip '{chip}' is now proven by {sorted(proofs[chip])[0]} — remove it from "
                "configs/ci/chip-coverage.yaml. The ledger is for gaps, not history."
            )

    if len(uncovered) > ceiling:
        errors.append(
            f"{len(uncovered)} chips are uncovered but the ceiling is {ceiling}. "
            "Cover the new chip rather than raising the ceiling."
        )
    if len(uncovered) < ceiling:
        errors.append(
            f"only {len(uncovered)} chips are uncovered and the ceiling is {ceiling}. "
            f"Lower `max_uncovered:` to {len(uncovered)} so the gap cannot silently grow back."
        )
    return proofs, uncovered, errors


def report(proofs: dict[str, set[str]], uncovered: list[str], entries: dict[str, dict]) -> None:
    chips = shipped_chips()
    harness = harness_proofs()
    print(f"## CLI chip coverage — {len(chips) - len(uncovered)}/{len(chips)} chips execute firmware in CI")
    print()
    print(f"{'CHIP':<20} {'STATE':<10} PROOF / REASON")
    print(f"{'----':<20} {'-----':<10} --------------")
    for chip in chips:
        if chip in proofs:
            print(f"{chip:<20} {'covered':<10} {', '.join(sorted(proofs[chip]))}")
            continue
        entry = entries.get(chip) or {}
        kind = entry.get("kind", "UNDECLARED")
        detail = entry.get("reason", "(no reason on file)")
        # What DOES run this chip today, so the row is a gap and not a blank.
        if chip in harness:
            detail += f" [in-process only: {', '.join(sorted(harness[chip]))}]"
        print(f"{chip:<20} {kind:<10} {detail}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--report", action="store_true", help="print the coverage table")
    ap.add_argument("--repo-root", help="grade this tree instead of the checkout (tests)")
    args = ap.parse_args()

    if args.repo_root:
        set_root(Path(args.repo_root))

    try:
        proofs, uncovered, errors = evaluate()
        entries, _ = load_ledger()
    except CoverageError as exc:
        print(f"::error::chip coverage: {exc}", file=sys.stderr)
        return 1

    if args.report:
        report(proofs, uncovered, entries)
        print()

    for err in errors:
        print(f"::error::{err}", file=sys.stderr)
    if errors:
        return 1

    print(
        f"chip coverage: {len(shipped_chips()) - len(uncovered)} chips proven by a CLI gate, "
        f"{len(uncovered)} declared uncovered (ceiling {len(uncovered)})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
