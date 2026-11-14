"""Layout gate for the one-file-per-entry registries, and the split helper."""

import subprocess
import sys
from pathlib import Path

import pytest
import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import registry_dir as rd  # noqa: E402
import registry_split as split  # noqa: E402

REPO = Path(__file__).resolve().parent.parent


def make(root: Path, files: dict[str, str]) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        p = root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    return root


GOOD = {"_index.yaml": "# header\nextra: 1\n", "a.yaml": "id: a\nx: 1\n", "b.yaml": "id: b\nx: 2\n"}


def test_loads_in_sorted_order_with_index_keys(tmp_path):
    reg = rd.load_yaml_registry(make(tmp_path / "d", GOOD))
    assert reg == {"extra": 1, "boards": [{"id": "a", "x": 1}, {"id": "b", "x": 2}]}


def test_comment_only_index_is_empty(tmp_path):
    d = make(tmp_path / "d", {**GOOD, "_index.yaml": "# only comments\n"})
    assert rd.load_yaml_registry(d) == {"boards": [{"id": "a", "x": 1}, {"id": "b", "x": 2}]}


@pytest.mark.parametrize(
    "files, needle",
    [
        ({**GOOD, "c.yaml": "id: other\n"}, "file name must be other.yaml"),
        ({**GOOD, "c.yaml": "id: a\n"}, "duplicate id 'a'"),
        ({**GOOD, "c.yaml": ""}, "empty"),
        ({**GOOD, "c.yaml": "# nothing but a comment\n"}, "empty"),
        ({**GOOD, "c.yaml": "id: [unclosed\n"}, "does not parse"),
        ({**GOOD, "c.yaml": "- id: c\n"}, "must be one mapping"),
        ({**GOOD, "c.yaml": "x: 1\n"}, "no string `id:`"),
        ({**GOOD, "notes.txt": "hi\n"}, "stray file"),
        ({**GOOD, "c.yml": "id: c\n"}, "stray file"),
        ({**GOOD, "sub/c.yaml": "id: c\n"}, "stray file"),
        ({"a.yaml": "id: a\n"}, "_index.yaml: missing"),
        ({**GOOD, "_index.yaml": "boards: []\n"}, "must not define `boards`"),
    ],
)
def test_bad_layouts_are_refused(tmp_path, files, needle):
    with pytest.raises(rd.RegistryError, match=needle):
        rd.load_yaml_registry(make(tmp_path / "d", files))


def test_missing_directory_is_refused(tmp_path):
    with pytest.raises(rd.RegistryError, match="does not exist"):
        rd.load_yaml_registry(tmp_path / "nope")


def test_committed_registries_are_valid():
    for rel, key in rd.COMMITTED.items():
        assert len(rd.load_yaml_registry(REPO / rel, key)[key]) > 10, rel


def test_cli_check_fails_on_a_bad_directory_and_passes_on_the_repo(tmp_path):
    bad = make(tmp_path / "d", {**GOOD, "c.yaml": "id: zzz\n"})
    run = lambda *a: subprocess.run(  # noqa: E731
        [sys.executable, str(REPO / "validation/registry_dir.py"), "--check", *a],
        capture_output=True, text=True,
    )
    assert run(str(bad)).returncode == 1
    assert run().returncode == 0


OLD = """\
# header comment
boards:
  # ── banner
  - id: one
    # note about one
    a: 1
    items:
      - "x"
  # a banner before two

  - id: two
    text: |
      block
        indented
    # trailing note for two

sketches:
  - id: s1
"""


def test_split_roundtrips_to_the_old_structure(tmp_path):
    header, entries, gaps, tail = split.split_text(OLD)
    d = tmp_path / "d"
    d.mkdir()
    for eid, lines in entries.items():
        (d / f"{eid}.yaml").write_text("\n".join(lines) + "\n")
    (d / "_index.yaml").write_text(split.render_index(header, gaps, tail))
    new = rd.load_yaml_registry(d)
    old = yaml.safe_load(OLD)
    assert {b["id"]: b for b in new["boards"]} == {b["id"]: b for b in old["boards"]}
    assert new["sketches"] == old["sketches"]
    assert "# note about one" in (d / "one.yaml").read_text()
    assert "banner" in (d / "_index.yaml").read_text()


def test_split_cli_extracts_one_entry(tmp_path):
    src = tmp_path / "old.yaml"
    src.write_text(OLD)
    out = tmp_path / "out"
    r = subprocess.run(
        [sys.executable, str(REPO / "scripts/registry_split.py"), "--registry", "manifest",
         "--from", str(src), "--id", "two", "--out", str(out)],
        capture_output=True, text=True,
    )
    assert r.returncode == 0, r.stderr
    assert sorted(p.name for p in out.iterdir()) == ["two.yaml"]
    assert yaml.safe_load((out / "two.yaml").read_text())["id"] == "two"
