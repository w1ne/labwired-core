"""Layout and schema gate for validation/bus_proof/ (one file per chip)."""

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bus_proof_matrix as bp  # noqa: E402
import registry_dir as rd  # noqa: E402

CELL = {"state": "none", "gap": "g"}


def entry(chip, **over):
    e = {"id": chip, "in_boards": False, "uart": dict(CELL), "spi": dict(CELL), "i2c": dict(CELL)}
    e.update(over)
    return e


def make(root: Path, entries: dict | None = None, extra: dict | None = None) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    idx = {"$schema_note": "n", "chip_set_derived_from": "c", "surveyed": "2026-01-01"}
    (root / "_index.json").write_text(bp.render(idx), encoding="utf-8")
    for chip, e in (entries if entries is not None else {"a": entry("a"), "b": entry("b")}).items():
        (root / f"{chip}.json").write_text(bp.render(bp.canonical_entry(e)), encoding="utf-8")
    for name, text in (extra or {}).items():
        (root / name).write_text(text, encoding="utf-8")
    return root


def test_committed_directory_is_valid_and_complete():
    bp.validate_directory()
    m = bp.load_matrix()
    assert set(m) == {"$schema_note", "chip_set_derived_from", "surveyed", "chips"}
    assert len(m["chips"]) > 100


def test_aggregate_shape(tmp_path):
    m = bp.load_matrix(make(tmp_path / "d"))
    assert list(m["chips"]) == ["a", "b"]
    assert m["chips"]["a"] == {"in_boards": False, "uart": CELL, "spi": CELL, "i2c": CELL}


def test_split_then_load_round_trips(tmp_path):
    legacy = {
        "$schema_note": "n",
        "chip_set_derived_from": "c",
        "surveyed": "s",
        "chips": {"zz": {k: v for k, v in entry("zz").items() if k != "id"},
                  "aa": {k: v for k, v in entry("aa").items() if k != "id"}},
    }
    d = tmp_path / "d"
    assert bp.split(json.dumps(legacy), d) == 2
    bp.validate_directory(d)
    assert bp.load_matrix(d) == legacy


@pytest.mark.parametrize(
    "mutate, needle",
    [
        (lambda e: e.update(extra_key=1), "unknown key 'extra_key'"),
        (lambda e: e["uart"].update(bogus="x"), "unknown key 'bogus'"),
        (lambda e: e["uart"].update(state="green"), "state must be one of"),
        (lambda e: e.pop("spi"), "missing `spi` cell"),
        (lambda e: e.update(in_boards="yes"), "in_boards must be true/false"),
    ],
)
def test_schema_violations_are_refused(tmp_path, mutate, needle):
    e = entry("a")
    mutate(e)
    d = make(tmp_path / "d", {"b": entry("b")})
    # write raw (not canonicalised) so the offending key survives
    (d / "a.json").write_text(json.dumps(e, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    with pytest.raises(rd.RegistryError, match=needle):
        bp.validate_directory(d)


def test_ordering_and_format_drift_is_refused(tmp_path):
    d = make(tmp_path / "d")
    e = entry("a")
    reordered = {k: e[k] for k in ("in_boards", "id", "uart", "spi", "i2c")}
    (d / "a.json").write_text(json.dumps(reordered, indent=2) + "\n", encoding="utf-8")
    with pytest.raises(rd.RegistryError, match="not in canonical form"):
        bp.validate_directory(d)
    (d / "a.json").write_text(json.dumps(e), encoding="utf-8")  # compact, no newline
    with pytest.raises(rd.RegistryError, match="not in canonical form"):
        bp.validate_directory(d)


@pytest.mark.parametrize(
    "extra, needle",
    [
        ({"c.json": bp.render(bp.canonical_entry(entry("other")))}, "file name must be other.json"),
        ({"notes.txt": "x"}, "stray file"),
        ({"c.yaml": "id: c\n"}, "stray file"),
        ({"c.json": "{"}, "does not parse"),
        ({"c.json": "[]\n"}, "must be one mapping"),
    ],
)
def test_layout_violations_are_refused(tmp_path, extra, needle):
    d = make(tmp_path / "d", extra=extra)
    with pytest.raises(rd.RegistryError, match=needle):
        rd.load_json_registry(d, "entries")


def test_missing_index_and_chips_key_in_index_refused(tmp_path):
    d = make(tmp_path / "d")
    (d / "_index.json").unlink()
    with pytest.raises(rd.RegistryError, match="_index.json: missing"):
        rd.load_json_registry(d, "entries")
    (d / "_index.json").write_text('{"entries": []}\n')
    with pytest.raises(rd.RegistryError, match="must not define `entries`"):
        rd.load_json_registry(d, "entries")
