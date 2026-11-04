#!/usr/bin/env python3
"""Every part we onboard must be exercised by a lab or a test, or say why not.

`configs/devices/*.yaml` is the onboarded part set: each file is a descriptor
the Playground offers as a part. Nothing asked whether any of them is ever
attached to a running board. `chip_coverage.py` answers that for CHIPS; for
parts the only lists were hand-kept (`lab-smoke-from-source.sh` names seven
labs inline), so a descriptor could land, be offered in the catalog, and be
attached by nothing at all — and nothing said so.

Coverage here is DERIVED, never declared, the same way chip_coverage.py does
it. The `type:` spellings come from `crates/config/src/embedded.rs` — the one
table the runtime resolves a manifest's `type:` through — so an alias resolves
to the same descriptor here as it does at attach time:

  * a part is LAB-exercised if a system manifest that an example lab ships
    (`examples/**`: either a manifest itself, or the `inputs.system:` of a test
    script there) lists it in `external_devices:`. That is the Playground
    claim: a user can open the lab and see the part run;
  * a part is TEST-exercised if an integration test under `crates/*/tests/`
    names one of its spellings as a `type:` (quoted, escaped inside a Rust
    string, or as a fixture manifest's YAML), or names a `configs/systems/`
    manifest that lists it. In-crate `#[cfg(test)]` unit tests do NOT count:
    most of them parse or construct a descriptor, which is not exercising it.

`configs/ci/part-coverage.yaml` is then only for what the derivation cannot
see, each entry with a reason:

  * `kind: tested` — a test DOES exercise the part but reaches it in a way the
    derivation does not read (a Rust constructor rather than a `type:` string).
    It must name the test files in `tests:`, and each must exist and mention
    the part, so the entry cannot outlive the test;
  * `kind: device-only` — nothing exercises it today, and `reason:` says why.

`max_device_only:` is a ceiling and `min_lab_exercised:` a floor, and both must
EQUAL today's count: the gap cannot grow back silently, and a lab deleted from
`examples/` cannot quietly drop a part from Playground-proven to test-only. A
part that becomes derivable must leave the ledger, so it cannot rot into a list
of excuses.

    python3 scripts/ci/part_coverage.py            # gate (exit 1 on drift)
    python3 scripts/ci/part_coverage.py --report   # show who exercises what
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


def device_dir() -> Path:
    return REPO_ROOT / "configs/devices"


def embedded_table() -> Path:
    return REPO_ROOT / "crates/config/src/embedded.rs"


def examples_dir() -> Path:
    return REPO_ROOT / "examples"


def ledger_path() -> Path:
    return REPO_ROOT / "configs/ci/part-coverage.yaml"


# One `EMBEDDED_DEVICES` row: `(&["spelling", ...], include_str!(".../<stem>.yaml"))`.
EMBED_ROW_RE = re.compile(
    r'&\[([^\]]*)\],\s*include_str!\("[./]*configs/devices/([\w.-]+)\.yaml"\)'
)
# A `configs/systems/<name>.yaml` path written in a test source.
SYSTEM_PATH_RE = re.compile(r"configs/systems/([\w.-]+\.yaml)")

KINDS = ("tested", "device-only")


class CoverageError(RuntimeError):
    """The ledger and the tree disagree about what is exercised."""


def load_yaml(path: Path) -> dict:
    return yaml.safe_load(path.read_text()) or {}


def onboarded_parts() -> list[str]:
    """Descriptor stems we ship, FROM DISK — never a hand-kept list."""
    return sorted(p.stem for p in device_dir().glob("*.yaml"))


def spellings() -> dict[str, str]:
    """`type:` spelling -> descriptor stem, from the runtime's own table.

    A descriptor on disk that the table does not embed cannot be attached by
    any manifest, so `evaluate` reports it rather than this guessing a name.
    """
    if not embedded_table().is_file():
        raise CoverageError("crates/config/src/embedded.rs (the type table) is missing")
    out: dict[str, str] = {}
    for names, stem in EMBED_ROW_RE.findall(embedded_table().read_text()):
        for name in re.findall(r'"([^"]+)"', names):
            out[name] = stem
    if not out:
        raise CoverageError("no EMBEDDED_DEVICES rows parsed from crates/config/src/embedded.rs")
    return out


def _manifest_types(path: Path) -> list[str]:
    """The `external_devices[].type` strings of a system manifest, or []."""
    if not path.is_file():
        return []
    try:
        data = yaml.safe_load(path.read_text())
    except yaml.YAMLError:
        return []
    if not isinstance(data, dict) or not isinstance(data.get("external_devices"), list):
        return []
    return [
        str(d["type"])
        for d in data["external_devices"]
        if isinstance(d, dict) and d.get("type")
    ]


def _rel(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def lab_proofs(names: dict[str, str]) -> dict[str, set[str]]:
    """part -> example-lab manifests that attach it.

    A lab's system can live outside `examples/` (`inputs.system:
    ../../configs/systems/<board>.yaml`); it still ships with the lab, so the
    script's system is followed wherever it points.
    """
    proofs: dict[str, set[str]] = {}
    manifests: set[Path] = set()
    for path in sorted(examples_dir().rglob("*.yaml")):
        if "target" in path.relative_to(examples_dir()).parts:
            continue
        manifests.add(path.resolve())
        try:
            data = yaml.safe_load(path.read_text())
        except yaml.YAMLError:
            continue
        system = (data.get("inputs") or {}).get("system") if isinstance(data, dict) else None
        if isinstance(system, str) and system:
            manifests.add((path.parent / system).resolve())
    for manifest in sorted(manifests):
        for t in _manifest_types(manifest):
            if t in names:
                proofs.setdefault(names[t], set()).add(_rel(manifest))
    return proofs


# A whole quoted string: `"x"` in Rust, or `\"x\"` inside a Rust string that
# holds YAML (the escape is outside the capture, so both read as `x`).
QUOTED_RE = re.compile(r'"([\w.:-]+)\\?"')
# A YAML `type:` value, quoted or not — fixtures and raw strings.
TYPE_VALUE_RE = re.compile(r"""\btype:\s*\\?["']?([\w.:-]+)""")


def _named_types(text: str) -> set[str]:
    """Every whole token `text` writes as a quoted string or a `type:` value.

    Whole tokens, not substrings: `ssd1306` must not count for `ssd1306_128x32`.
    One scan per file, intersected with the spelling table by the caller — a
    regex per spelling per file is ~150x the work for the same answer.
    """
    return set(QUOTED_RE.findall(text)) | set(TYPE_VALUE_RE.findall(text))


def integration_sources() -> list[Path]:
    """Integration-test sources and their fixtures, under `crates/*/tests/`."""
    return sorted(
        p
        for p in REPO_ROOT.glob("crates/*/tests/**/*")
        if p.is_file() and p.suffix in (".rs", ".yaml", ".yml")
    )


def integration_proofs(names: dict[str, str]) -> dict[str, set[str]]:
    """part -> integration tests (or fixtures) that name it as a `type:`."""
    proofs: dict[str, set[str]] = {}
    for path in integration_sources():
        text = path.read_text(errors="replace")
        for spelling in _named_types(text) & names.keys():
            proofs.setdefault(names[spelling], set()).add(_rel(path))
        # A test that loads a shared board manifest exercises what it attaches.
        for system in set(SYSTEM_PATH_RE.findall(text)):
            for t in _manifest_types(REPO_ROOT / "configs/systems" / system):
                if t in names:
                    proofs.setdefault(names[t], set()).add(
                        f"{_rel(path)} (via configs/systems/{system})"
                    )
    return proofs


def load_ledger() -> tuple[dict[str, dict], int, int]:
    """(entries, max_device_only, min_lab_exercised).

    No file means nothing declared, and that fails CLOSED: every underived part
    is then an error, so deleting the ledger makes the gate stricter.
    """
    if not ledger_path().is_file():
        return {}, 0, 0
    data = load_yaml(ledger_path())
    entries = data.get("parts") or {}
    ceiling = data.get("max_device_only")
    floor = data.get("min_lab_exercised")
    if not isinstance(ceiling, int) or not isinstance(floor, int):
        raise CoverageError(
            "part-coverage.yaml needs integer `max_device_only:` and `min_lab_exercised:`"
        )
    return entries, ceiling, floor


def evaluate() -> tuple[dict, dict[str, list[str]], list[str]]:
    """Return ({"lab", "test"} proofs, {state: parts}, errors).

    States: `lab` and `test` are derived; `tested` and `device-only` come from
    the ledger; `gap` is everything else, and every gap is an error.
    """
    parts = onboarded_parts()
    names = spellings()
    labs = lab_proofs(names)
    tests = integration_proofs(names)
    entries, ceiling, floor = load_ledger()

    errors: list[str] = []
    embedded = set(names.values())
    for part in parts:
        if part not in embedded:
            errors.append(
                f"part '{part}' has configs/devices/{part}.yaml but no EMBEDDED_DEVICES row in "
                "crates/config/src/embedded.rs, so no manifest can attach it. Add the row."
            )

    states: dict[str, list[str]] = {k: [] for k in ("lab", "test", "tested", "device-only", "gap")}
    for part in parts:
        entry = entries.get(part)
        if part in labs or part in tests:
            states["lab" if part in labs else "test"].append(part)
            if entry is not None:
                where = sorted(labs.get(part) or tests[part])[0]
                errors.append(
                    f"part '{part}' is now exercised by {where} — remove it from "
                    "configs/ci/part-coverage.yaml. The ledger is for gaps, not history."
                )
            continue
        if entry is None:
            states["gap"].append(part)
            errors.append(
                f"part '{part}' (configs/devices/{part}.yaml) is onboarded but NOTHING exercises "
                "it: no example lab attaches it and no test under crates/*/tests/ names its "
                "type. Add a lab in examples/ whose system.yaml lists it under "
                "external_devices:, or an integration test that attaches it — or, if it "
                "genuinely cannot run yet, add it to configs/ci/part-coverage.yaml as "
                "`kind: device-only` with a `reason:`."
            )
            continue
        kind = entry.get("kind")
        if kind not in KINDS:
            errors.append(f"part '{part}': ledger `kind` must be one of {KINDS}")
            states["gap"].append(part)
            continue
        if not str(entry.get("reason") or "").strip():
            errors.append(f"part '{part}': ledger entry needs a concrete `reason`")
        if kind == "tested":
            named = entry.get("tests") or []
            if not named:
                errors.append(f"part '{part}': `kind: tested` must name its `tests:`")
            for rel in named:
                path = REPO_ROOT / str(rel)
                if not path.is_file():
                    errors.append(f"part '{part}': named test {rel} does not exist")
                elif part not in path.read_text(errors="replace"):
                    errors.append(f"part '{part}': named test {rel} never mentions '{part}'")
        states[kind].append(part)

    for part in sorted(entries):
        if part not in parts:
            errors.append(
                f"ledger lists '{part}', which is not in configs/devices/ — drop the stale entry"
            )

    n_device_only = len(states["device-only"])
    if n_device_only > ceiling:
        errors.append(
            f"{n_device_only} parts are device-only but the ceiling is {ceiling}. "
            "Exercise the new part rather than raising `max_device_only:`."
        )
    elif n_device_only < ceiling:
        errors.append(
            f"only {n_device_only} parts are device-only and the ceiling is {ceiling}. "
            f"Lower `max_device_only:` to {n_device_only} so the gap cannot silently grow back."
        )
    n_lab = len(states["lab"])
    if n_lab < floor:
        errors.append(
            f"only {n_lab} parts are attached by an example lab but the floor is {floor}. "
            "A lab stopped attaching a part (deleted, or its system.yaml changed); restore "
            "it rather than lowering `min_lab_exercised:`."
        )
    elif n_lab > floor:
        errors.append(
            f"{n_lab} parts are now attached by an example lab and the floor is {floor}. "
            f"Raise `min_lab_exercised:` to {n_lab} so the gain cannot silently go away."
        )
    return {"lab": labs, "test": tests}, states, errors


def report(proofs: dict, states: dict[str, list[str]], entries: dict[str, dict]) -> None:
    parts = onboarded_parts()
    exercised = len(states["lab"]) + len(states["test"]) + len(states["tested"])
    print(
        f"## Part coverage — {exercised}/{len(parts)} exercised "
        f"({len(states['lab'])} by an example lab, "
        f"{len(states['test']) + len(states['tested'])} by a test only), "
        f"{len(states['device-only'])} device-only, {len(states['gap'])} gap"
    )
    print()
    print(f"{'PART':<26} {'STATE':<12} PROOF / REASON")
    print(f"{'----':<26} {'-----':<12} --------------")
    for part in parts:
        state = next(s for s, members in states.items() if part in members)
        if state in ("lab", "test"):
            where = sorted(proofs[state][part])
            detail = ", ".join(where[:2]) + (f" (+{len(where) - 2})" if len(where) > 2 else "")
        elif state == "gap":
            detail = "UNDECLARED — nothing exercises it"
        else:
            entry = entries.get(part) or {}
            detail = " ".join(str(entry.get("reason", "")).split())
            if state == "tested":
                detail += f" [{', '.join(entry.get('tests') or [])}]"
        print(f"{part:<26} {state:<12} {detail}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--report", action="store_true", help="print the coverage table")
    ap.add_argument("--repo-root", help="grade this tree instead of the checkout (tests)")
    args = ap.parse_args()

    if args.repo_root:
        set_root(Path(args.repo_root))

    try:
        proofs, states, errors = evaluate()
        entries, _, _ = load_ledger()
    except CoverageError as exc:
        print(f"::error::part coverage: {exc}", file=sys.stderr)
        return 1

    if args.report:
        report(proofs, states, entries)
        print()

    for err in errors:
        print(f"::error::{err}", file=sys.stderr)

    print(
        f"part coverage: {len(onboarded_parts())} onboarded — "
        f"{len(states['lab'])} lab-exercised, "
        f"{len(states['test']) + len(states['tested'])} test-exercised, "
        f"{len(states['device-only'])} device-only, {len(states['gap'])} gap"
    )
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
