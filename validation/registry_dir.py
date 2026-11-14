#!/usr/bin/env python3
"""Shared loader for registries stored as one file per entry.

A registry that used to be one list in one file (`validation/manifest.yaml`,
`validation/arduino-matrix/boards.yaml`, `validation/bus_proof_matrix.json`) is
now a directory:

    <dir>/_index.yaml     everything that is not an entry (header comments,
                          sibling top-level keys such as `sketches:`)
    <dir>/<entry-id>.yaml exactly one entry, as a top-level mapping

`validation/bus_proof` is the same layout with `.json` files (`_index.json`,
`<chip>.json`); its entry schema is checked by `validation/bus_proof_matrix.py`,
which `--check` also runs.

`load_yaml_registry` returns the structure the single file used to parse to:
`{**index_keys, "<list_key>": [entry, ...]}`, entries in sorted file-name order.

Order: no consumer depends on list order. The manifest report sorts its rows
itself; the Arduino matrix, fleet verdict and coverage gate treat the boards as
a set keyed by id. Sorted file-name order only makes the result deterministic.

The loader refuses (RegistryError, every problem listed at once):
  * a file whose name is not `<id>.yaml` (or whose `id` is missing/not a string)
  * duplicate ids across files
  * a file that does not parse, parses to nothing, or is not a mapping
  * any stray file or directory that is not `_index.yaml` or `<id>.yaml`
  * a missing `_index.yaml`, or an index that itself defines the list key

Command line:  python3 validation/registry_dir.py --check [DIR ...]
checks the committed registry directories (all of them when none is named).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - environment precondition
    print("ERROR: PyYAML required: pip install pyyaml", file=sys.stderr)
    sys.exit(2)

INDEX_NAME = "_index.yaml"
SUFFIX = ".yaml"
JSON_INDEX_NAME = "_index.json"
JSON_SUFFIX = ".json"

REPO_ROOT = Path(__file__).resolve().parent.parent

# directory (repo-relative) -> list key
COMMITTED = {
    "validation/boards": "boards",
    "validation/arduino-matrix/boards": "boards",
    "validation/bus_proof": "entries",
}

# directories whose files are JSON rather than YAML
JSON_DIRS = {"validation/bus_proof"}


class RegistryError(ValueError):
    """The registry directory is not a valid one-file-per-entry registry."""


def entry_files(directory: Path, index_name: str = INDEX_NAME) -> list[Path]:
    """Sorted entry files of `directory` (everything but the index)."""
    return sorted(p for p in directory.iterdir() if p.name != index_name)


def _json_load(text: str, Loader=None):
    return json.loads(text)


def load_json_registry(directory: Path, list_key: str = "entries", *, id_key: str = "id") -> dict:
    """Same contract as `load_yaml_registry`, for `_index.json` + `<id>.json`."""
    return load_yaml_registry(
        directory,
        list_key,
        id_key=id_key,
        suffix=JSON_SUFFIX,
        index_name=JSON_INDEX_NAME,
        _parse=_json_load,
        _parse_errors=(json.JSONDecodeError,),
    )


def load_yaml_registry(
    directory: Path,
    list_key: str = "boards",
    *,
    yaml_loader=yaml.SafeLoader,
    id_key: str = "id",
    suffix: str = SUFFIX,
    index_name: str = INDEX_NAME,
    _parse=None,
    _parse_errors=(yaml.YAMLError, ValueError),
) -> dict:
    parse = _parse or yaml.load
    directory = Path(directory)
    if not directory.is_dir():
        raise RegistryError(f"{directory}: registry directory does not exist")

    errors: list[str] = []

    index: dict = {}
    index_path = directory / index_name
    if not index_path.is_file():
        errors.append(f"{index_path}: missing (header and non-entry keys live here)")
    else:
        try:
            data = parse(index_path.read_text(encoding="utf-8"), Loader=yaml_loader)
        except _parse_errors as exc:
            errors.append(f"{index_path}: does not parse: {exc}")
            data = None
        if data is None:
            data = {}
        if not isinstance(data, dict):
            errors.append(f"{index_path}: must be a mapping (or comments only)")
        elif list_key in data:
            errors.append(f"{index_path}: must not define `{list_key}`; entries are files")
        else:
            index = data

    entries: list[dict] = []
    first_seen: dict[str, str] = {}
    for path in entry_files(directory, index_name):
        if not path.is_file() or path.suffix != suffix:
            errors.append(
                f"{path}: stray file; only {index_name} and <{id_key}>{suffix} are allowed here"
            )
            continue
        try:
            entry = parse(path.read_text(encoding="utf-8"), Loader=yaml_loader)
        except _parse_errors as exc:
            errors.append(f"{path}: does not parse: {exc}")
            continue
        if entry is None:
            errors.append(f"{path}: empty (no entry in it)")
            continue
        if not isinstance(entry, dict):
            errors.append(f"{path}: must be one mapping (a single entry), got {type(entry).__name__}")
            continue
        eid = entry.get(id_key)
        if not isinstance(eid, str) or not eid:
            errors.append(f"{path}: has no string `{id_key}:`")
            continue
        if path.stem != eid:
            errors.append(f"{path}: file name must be {eid}{suffix} (entry {id_key} is {eid!r})")
        if eid in first_seen:
            errors.append(f"{path}: duplicate {id_key} {eid!r} (also in {first_seen[eid]})")
        else:
            first_seen[eid] = path.name
        entries.append(entry)

    if errors:
        raise RegistryError(f"{directory}: invalid registry:\n  " + "\n  ".join(errors))

    out = dict(index)
    out[list_key] = entries
    return out


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if args and args[0] == "--check":
        args = args[1:]
    elif args:
        print(__doc__)
        return 2
    targets = [Path(a) for a in args] or [REPO_ROOT / d for d in COMMITTED]
    rc = 0
    for target in targets:
        key = COMMITTED.get(target.resolve().relative_to(REPO_ROOT).as_posix(), "boards") \
            if target.resolve().is_relative_to(REPO_ROOT) else "boards"
        is_json = target.resolve().is_relative_to(REPO_ROOT) and \
            target.resolve().relative_to(REPO_ROOT).as_posix() in JSON_DIRS
        try:
            reg = load_json_registry(target, key) if is_json else load_yaml_registry(target, key)
            if is_json:
                import bus_proof_matrix

                bus_proof_matrix.validate_directory(target)
        except RegistryError as exc:
            print(f"ERROR: {exc}", file=sys.stderr)
            rc = 1
            continue
        print(f"ok: {target}: {len(reg[key])} entries")
    return rc


if __name__ == "__main__":
    sys.exit(main())
