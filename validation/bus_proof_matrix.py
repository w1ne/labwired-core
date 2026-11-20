#!/usr/bin/env python3
"""Per-chip UART/SPI/I2C evidence matrix, stored one file per chip.

The matrix used to be one file, `validation/bus_proof_matrix.json`, edited by
every chip PR. It is now a directory:

    validation/bus_proof/_index.json    the three non-chip keys of the old file
                                        (`$schema_note`, `chip_set_derived_from`,
                                        `surveyed`)
    validation/bus_proof/<chip>.json    one chip: {"id": "<chip>", "in_boards": ..,
                                        "uart": {..}, "spi": {..}, "i2c": {..}}

`load_matrix()` aggregates the directory back into the structure the single
file parsed to, `{**index, "chips": {<chip>: {in_boards, uart, spi, i2c}}}`; the
Rust gate (crates/core/src/tests/bus_proof_matrix.rs) reads the same layout
with the same rules. Verdict rules (what `proven` needs, the ratchet against
the merge base) stay in that Rust file; this module only guards the storage.

`--check` (also run by `validation/registry_dir.py --check`) refuses:
  * a file whose name is not `<chip>.json` or whose `id` differs, duplicate ids,
    stray files, unparsable files (registry_dir.py)
  * unknown keys in an entry, a bus cell, or the index; a missing bus; a
    `state` that is not proven/shallow/none
  * ordering drift: every file must be byte-identical to its canonical
    rendering (fixed key order, 2-space indent, UTF-8, trailing newline), so a
    hand edit that reorders keys or reformats is rejected, not silently kept

Other commands:
  --dump                 print the aggregate as canonical JSON
  --equals-legacy FILE   exit 0 iff the aggregate equals the old single file
                         (parsed, so key order and whitespace do not count)
  --split FILE           write validation/bus_proof/ from an old single file; for
                         a branch that edited the old file before the split:
                           git show HEAD:validation/bus_proof_matrix.json > /tmp/m.json
                           python3 validation/bus_proof_matrix.py --split /tmp/m.json
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import registry_dir as rd  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent
DIR = REPO_ROOT / "validation" / "bus_proof"

INDEX_KEYS = ("$schema_note", "chip_set_derived_from", "surveyed")
ENTRY_KEYS = ("id", "in_boards", "uart", "spi", "i2c")
BUSES = ("uart", "spi", "i2c")
CELL_KEYS = ("state", "file", "test", "signal", "gap", "lane", "note")
STATES = ("proven", "shallow", "none")


def _canonical_cell(cell: dict) -> dict:
    return {k: cell[k] for k in CELL_KEYS if k in cell}


def canonical_entry(entry: dict) -> dict:
    out = {}
    for k in ENTRY_KEYS:
        if k in entry:
            out[k] = _canonical_cell(entry[k]) if k in BUSES and isinstance(entry[k], dict) else entry[k]
    return out


def render(obj: dict) -> str:
    return json.dumps(obj, indent=2, ensure_ascii=False) + "\n"


def validate_directory(directory: Path = DIR) -> None:
    """Schema, unknown-key and canonical-form checks. Raises RegistryError."""
    directory = Path(directory)
    errors: list[str] = []
    index_path = directory / rd.JSON_INDEX_NAME
    if index_path.is_file():
        idx = json.loads(index_path.read_text(encoding="utf-8"))
        for k in idx:
            if k not in INDEX_KEYS:
                errors.append(f"{index_path}: unknown key {k!r}")
        for k in INDEX_KEYS:
            if k not in idx:
                errors.append(f"{index_path}: missing key {k!r}")
        want = render({k: idx[k] for k in INDEX_KEYS if k in idx})
        if index_path.read_text(encoding="utf-8") != want:
            errors.append(f"{index_path}: not in canonical form (key order {list(INDEX_KEYS)}, indent 2)")
    for path in rd.entry_files(directory, rd.JSON_INDEX_NAME):
        if not path.is_file() or path.suffix != rd.JSON_SUFFIX:
            continue  # registry_dir reports stray files
        text = path.read_text(encoding="utf-8")
        entry = json.loads(text)
        if not isinstance(entry, dict):
            continue  # registry_dir reports non-mappings
        for k, v in entry.items():
            if k not in ENTRY_KEYS:
                errors.append(f"{path}: unknown key {k!r} (allowed: {list(ENTRY_KEYS)})")
            elif k == "in_boards" and not isinstance(v, bool):
                errors.append(f"{path}: in_boards must be true/false")
        for bus in BUSES:
            cell = entry.get(bus)
            if not isinstance(cell, dict):
                errors.append(f"{path}: missing `{bus}` cell")
                continue
            for k in cell:
                if k not in CELL_KEYS:
                    errors.append(f"{path}: {bus}: unknown key {k!r} (allowed: {list(CELL_KEYS)})")
            if cell.get("state") not in STATES:
                errors.append(f"{path}: {bus}: state must be one of {list(STATES)}")
            for k, v in cell.items():
                if not isinstance(v, str):
                    errors.append(f"{path}: {bus}.{k} must be a string")
        if text != render(canonical_entry(entry)):
            errors.append(
                f"{path}: not in canonical form (key order {list(ENTRY_KEYS)} / cell order "
                f"{list(CELL_KEYS)}, indent 2, UTF-8, trailing newline)"
            )
    if errors:
        raise rd.RegistryError(f"{directory}: invalid bus-proof entries:\n  " + "\n  ".join(errors))


def load_matrix(directory: Path = DIR) -> dict:
    """The structure `validation/bus_proof_matrix.json` used to parse to."""
    reg = rd.load_json_registry(directory, "entries")
    out = {k: v for k, v in reg.items() if k != "entries"}
    out["chips"] = {e["id"]: {k: v for k, v in e.items() if k != "id"} for e in reg["entries"]}
    return out


def split(legacy_text: str, directory: Path = DIR) -> int:
    legacy = json.loads(legacy_text)
    directory.mkdir(parents=True, exist_ok=True)
    (directory / rd.JSON_INDEX_NAME).write_text(
        render({k: legacy[k] for k in INDEX_KEYS}), encoding="utf-8"
    )
    extra = set(legacy) - set(INDEX_KEYS) - {"chips"}
    if extra:
        raise rd.RegistryError(f"legacy file has unexpected top-level keys: {sorted(extra)}")
    for chip, row in legacy["chips"].items():
        (directory / f"{chip}{rd.JSON_SUFFIX}").write_text(
            render(canonical_entry({"id": chip, **row})), encoding="utf-8"
        )
    return len(legacy["chips"])


def main(argv: list[str]) -> int:
    try:
        if argv[:1] == ["--check"]:
            validate_directory()
            rd.load_json_registry(DIR, "entries")
            print(f"ok: {DIR}: {len(load_matrix()['chips'])} chips")
            return 0
        if argv[:1] == ["--dump"]:
            sys.stdout.write(render(load_matrix()))
            return 0
        if argv[:1] == ["--equals-legacy"] and len(argv) == 2:
            old = json.loads(Path(argv[1]).read_text(encoding="utf-8"))
            new = load_matrix()
            if old == new:
                cells = sum(1 for r in new["chips"].values() for b in BUSES if b in r)
                print(f"IDENTICAL: {len(new['chips'])} chips, {cells} bus cells, "
                      f"top-level keys {sorted(new)}")
                return 0
            for k in sorted(set(old) | set(new)):
                if old.get(k) != new.get(k):
                    print(f"DIFFERS at top-level key {k!r}", file=sys.stderr)
            if old.get("chips") != new.get("chips"):
                oc, nc = old.get("chips", {}), new.get("chips", {})
                for c in sorted(set(oc) | set(nc)):
                    if oc.get(c) != nc.get(c):
                        print(f"  chip {c}: differs", file=sys.stderr)
            return 1
        if argv[:1] == ["--split"] and len(argv) == 2:
            n = split(Path(argv[1]).read_text(encoding="utf-8"))
            print(f"wrote {n} chip files + {rd.JSON_INDEX_NAME} to {DIR}")
            return 0
    except rd.RegistryError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
