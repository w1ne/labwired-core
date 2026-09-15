#!/usr/bin/env python3
"""Gate the deliberately-faked surface: FIDELITY.md's `CHEAT(...)` markers.

WHY THIS EXISTS

FIDELITY.md defines a seven-category taxonomy for behaviour the twin fakes and
tells the reader to find it with `grep -rn "CHEAT("`. A convention nothing
counts is a comment. This counts it.

WHAT IT IS NOT. It is not the only ledger of this debt, and pretending otherwise
was the actual defect. Three exist and none knew about the others:

  FIDELITY.md `CHEAT(...)` markers          the prose taxonomy
  `declared_thunk_symbols()`                 ratcheted at a hard ceiling in
                                             arduino_esp32_profile.rs
  `extract_arduino_esp32_thunks()`           the loader's symbol table

A reader could not reconcile 25 markers against a ceiling of 56 against a table
of ~187 names. `--reconcile` prints all three side by side so the numbers stop
disagreeing silently. Closing the gap between them is separate, sequenced work
(FIDELITY.md, Batches A-D); this makes the gap VISIBLE and COUNTED.

WHAT IT ASSERTS

  1. schema     every marker's category is one FIDELITY.md documents
  2. format     `CHEAT(CAT): <what is faked> - real: <what silicon does>`,
                with `, module` for a `//!` blanket over a whole file
  3. references every repo path FIDELITY.md names exists; every "see the
                CHEAT(X)" citation resolves to a real marker in its own file;
                every `file.rs:NN (CHEAT(...))` citation in validation/ lands on
                a marker line
  4. ratchet    the count per category only falls, against a baseline keyed on
                CONTENT, never on file:line

WHY THE BASELINE IS CONTENT-KEYED. A `file:line` key rots on the first move or
reflow -- which is exactly how FIDELITY.md came to name a path that no longer
exists and how a marker came to cite a `CHEAT(SKIP)` that is not in its file.
The key is a digest of the category plus the normalised "what is faked" clause,
so a rename or a move changes nothing and editing what is faked changes the key,
which should require review. `file` and `symbol` are stored alongside as
informational fields the tool rewrites on every run, so they self-heal.

Usage:
    cheat_ratchet.py --check        # the gate
    cheat_ratchet.py --write        # accept the current surface as the baseline
    cheat_ratchet.py --reconcile    # print the three ledgers side by side
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import date
from pathlib import Path

CORE_ROOT = Path(__file__).resolve().parents[2]
FIDELITY = CORE_ROOT / "FIDELITY.md"
BASELINE = CORE_ROOT / "validation" / "cheat_baseline.json"
VALIDATION_DIR = CORE_ROOT / "validation"

# A MARKER declares a cheat and is followed by a colon. A CITATION refers to one
# and is not. Conflating them is why "25 markers" was never a reproducible
# number: prose mentions and cross-references counted as cheats.
MARKER_RE = re.compile(r"CHEAT\(([A-Z][A-Z0-9-]*)(,\s*module)?\):")
CITATION_RE = re.compile(r"CHEAT\(([A-Z][A-Z0-9-]*)\)(?!\s*[,:])")
COMMENT_RE = re.compile(r"^\s*(//!|///|//)")
# The taxonomy table in FIDELITY.md: | `CHEAT(X)` | ... |
TAXONOMY_RE = re.compile(r"^\|\s*`CHEAT\(([A-Z][A-Z0-9-]*)\)`\s*\|")


def documented_categories() -> set[str]:
    """Parse the taxonomy out of FIDELITY.md. Never hardcode it here."""
    return {m.group(1) for line in FIDELITY.read_text().splitlines() if (m := TAXONOMY_RE.match(line))}


def rust_files() -> list[Path]:
    return sorted(
        p
        for p in (CORE_ROOT / "crates").rglob("*.rs")
        if "/target/" not in str(p)
    )


def normalise(text: str) -> str:
    """The clause, with whitespace and comment leaders removed.

    Keyed on this so a reflow or a rename does not invalidate a baseline entry,
    and so editing WHAT IS FAKED does.
    """
    return re.sub(r"\s+", " ", re.sub(r"^\s*(//!|///|//)\s*", "", text)).strip().rstrip(".")


def enclosing_symbol(lines: list[str], idx: int) -> str:
    """Best-effort name of the item the marker sits on. Informational only."""
    for j in range(idx, min(idx + 12, len(lines))):
        m = re.match(r"\s*(pub\s+)?(unsafe\s+)?(extern\s+\"[^\"]*\"\s+)?fn\s+([A-Za-z0-9_]+)", lines[j])
        if m:
            return m.group(4)
    return "<module>"


def collect_markers() -> tuple[list[dict], list[str]]:
    """Return (markers, format errors)."""
    markers: list[dict] = []
    errors: list[str] = []
    for path in rust_files():
        rel = path.relative_to(CORE_ROOT).as_posix()
        lines = path.read_text(errors="ignore").splitlines()
        for i, line in enumerate(lines):
            m = MARKER_RE.search(line)
            if not m:
                continue
            if not COMMENT_RE.match(line):
                # A marker inside a string literal is documentation ABOUT the
                # convention, not a declaration of a cheat.
                continue
            # The clause may wrap onto continuation comment lines.
            clause_lines = [line[m.end():]]
            for j in range(i + 1, len(lines)):
                nxt = lines[j]
                if not COMMENT_RE.match(nxt) or MARKER_RE.search(nxt):
                    break
                stripped = normalise(nxt)
                if not stripped:
                    break
                clause_lines.append(nxt)
            # Strip the comment leader from EACH line before joining, or the
            # `//` of a continuation line lands in the middle of the clause and
            # becomes part of the content key.
            clause = normalise(" ".join(normalise(c) for c in clause_lines))
            entry = {
                "category": m.group(1),
                "module_scope": bool(m.group(2)),
                "file": rel,
                "line": i + 1,
                "symbol": enclosing_symbol(lines, i),
                "clause": clause,
            }
            if "real:" not in clause.lower():
                errors.append(
                    f"{rel}:{i + 1}: CHEAT({m.group(1)}) has no `real:` clause — "
                    "state what silicon or real execution does instead"
                )
            markers.append(entry)
    return markers, errors


def key_of(entry: dict) -> str:
    return hashlib.sha256(f"{entry['category']}\n{entry['clause']}".encode()).hexdigest()[:16]


def check_schema(markers: list[dict]) -> list[str]:
    documented = documented_categories()
    if not documented:
        return ["FIDELITY.md taxonomy table did not parse — the schema is unknown, so nothing can be graded"]
    return [
        f"{e['file']}:{e['line']}: CHEAT({e['category']}) is not a category FIDELITY.md documents "
        f"({', '.join(sorted(documented))})"
        for e in markers
        if e["category"] not in documented
    ]


def check_references(markers: list[dict]) -> list[str]:
    """The rot check. All three of these were broken when this landed."""
    errors: list[str] = []
    text = FIDELITY.read_text()

    # (a) every repo path FIDELITY.md names must exist
    for m in re.finditer(r"`?((?:crates|scripts|configs|validation|docs|examples)/[A-Za-z0-9_./-]+\.[a-z]+)`?", text):
        rel = m.group(1)
        if not (CORE_ROOT / rel).exists():
            line = text[: m.start()].count("\n") + 1
            errors.append(f"FIDELITY.md:{line}: names {rel}, which does not exist")

    # (b) a "see the CHEAT(X)" citation must resolve inside its own file
    by_file: dict[str, set[str]] = {}
    for e in markers:
        by_file.setdefault(e["file"], set()).add(e["category"])
    for path in rust_files():
        rel = path.relative_to(CORE_ROOT).as_posix()
        for i, line in enumerate(path.read_text(errors="ignore").splitlines()):
            if not COMMENT_RE.match(line):
                continue
            for c in CITATION_RE.finditer(line):
                if c.group(1) not in by_file.get(rel, set()):
                    errors.append(
                        f"{rel}:{i + 1}: cites CHEAT({c.group(1)}), which is not declared in this file"
                    )

    # (c) `file.rs:NN (CHEAT(...))` citations in validation/ must land on a marker
    marker_lines = {(e["file"], e["line"]) for e in markers}
    for yml in sorted(VALIDATION_DIR.glob("*.yaml")):
        for i, line in enumerate(yml.read_text(errors="ignore").splitlines()):
            for c in re.finditer(r'"?([A-Za-z0-9_/]+\.rs):(\d+)[^"]*CHEAT\(', line):
                suffix, num = c.group(1), int(c.group(2))
                hits = [f for f, ln in marker_lines if f.endswith(suffix) and ln == num]
                if not hits:
                    errors.append(
                        f"{yml.name}:{i + 1}: cites {suffix}:{num} as a CHEAT marker; "
                        "no marker is on that line (line-number citations rot — cite the symbol)"
                    )
    return errors


def load_baseline() -> dict:
    if not BASELINE.exists():
        return {"entries": {}, "waived": []}
    return json.loads(BASELINE.read_text())


def check_ratchet(markers: list[dict], today: date) -> list[str]:
    base = load_baseline()
    errors: list[str] = []

    for w in base.get("waived", []):
        expires = date.fromisoformat(w["expires"])
        if today > expires:
            errors.append(
                f"waiver for {w['key']} expired {w['expires']} ({w.get('reason', 'no reason recorded')})"
            )

    waived = {w["key"] for w in base.get("waived", [])}
    known = set(base.get("entries", {})) | waived
    for e in markers:
        k = key_of(e)
        if k not in known:
            errors.append(
                f"{e['file']}:{e['line']}: NEW cheat CHEAT({e['category']}) — {e['clause'][:90]}\n"
                f"       If this is deliberate, run `scripts/ci/cheat_ratchet.py --write` and say why in "
                f"the commit message. The count is allowed to fall, never to rise unreviewed."
            )
    return errors


def write_baseline(markers: list[dict]) -> None:
    base = load_baseline()
    entries = {}
    for e in sorted(markers, key=lambda x: (x["file"], x["line"])):
        entries[key_of(e)] = {
            "category": e["category"],
            "clause": e["clause"],
            # Informational. Rewritten every run, so a move or rename self-heals
            # instead of rotting the way FIDELITY.md's paths did.
            "file": e["file"],
            "symbol": e["symbol"],
        }
    BASELINE.write_text(
        json.dumps({"entries": entries, "waived": base.get("waived", [])}, indent=2, sort_keys=True) + "\n"
    )
    print(f"wrote {BASELINE.relative_to(CORE_ROOT)} — {len(entries)} cheats")


def reconcile(markers: list[dict]) -> None:
    """Print the three ledgers side by side. A report, not a gate."""
    profile = CORE_ROOT / "crates/core/src/system/xtensa/arduino_esp32_profile.rs"
    loader = CORE_ROOT / "crates/loader/src/lib.rs"

    ceiling = "?"
    if profile.exists():
        m = re.search(r"CEILING:\s*usize\s*=\s*(\d+)", profile.read_text())
        if m:
            ceiling = m.group(1)
    loader_syms = "?"
    if loader.exists():
        body = loader.read_text()
        m = re.search(r"fn extract_arduino_esp32_thunks.*?\n}", body, re.S)
        if m:
            loader_syms = str(len(set(re.findall(r'"([a-z_][A-Za-z0-9_]*)"', m.group(0)))))

    print("three ledgers of the same debt:")
    print(f"  FIDELITY.md CHEAT markers            {len(markers)}")
    print(f"  declared_thunk_symbols() ceiling     {ceiling}")
    print(f"  extract_arduino_esp32_thunks symbols {loader_syms}")
    print()
    counts: dict[str, int] = {}
    for e in markers:
        counts[e["category"]] = counts.get(e["category"], 0) + 1
    for cat in sorted(counts):
        print(f"  CHEAT({cat}){'':<{max(0, 12 - len(cat))}} {counts[cat]}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Gate the CHEAT(...) fidelity-debt surface.")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--reconcile", action="store_true")
    args = ap.parse_args(argv)

    markers, format_errors = collect_markers()

    if args.reconcile:
        reconcile(markers)
        return 0
    if args.write:
        write_baseline(markers)
        return 0

    errors = format_errors + check_schema(markers) + check_references(markers)
    errors += check_ratchet(markers, date.today())

    print(f"{len(markers)} CHEAT markers across {len({e['file'] for e in markers})} files")
    if errors:
        for e in errors:
            print(f"::error::{e}", file=sys.stderr)
        print(f"\n{len(errors)} problem(s). See FIDELITY.md.", file=sys.stderr)
        return 1
    print("cheat ratchet: schema, references and count are honest")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
