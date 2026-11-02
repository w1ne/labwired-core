"""Tests for scripts/generate_chip_catalog.py and generate_chip_catalog_page.py.

The chip list page is generated from two catalogues; these tests pin the parts
that must not silently break:
  * a private part never leaks its descriptor contents into the output;
  * a retired part renders as excluded instead of supported;
  * a public part keeps its conformance level;
  * the page carries every chip as a row.

Run: python3 -m pytest -q scripts/test_generate_chip_catalog.py
"""
from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

yaml = pytest.importorskip("yaml")

ROOT = Path(__file__).resolve().parents[1]


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


catalog = _load("generate_chip_catalog", ROOT / "scripts/generate_chip_catalog.py")
page = _load("generate_chip_catalog_page", ROOT / "scripts/generate_chip_catalog_page.py")


@pytest.fixture()
def fake_tree(tmp_path: Path):
    """A minimal core + private tree with one chip on each side."""
    core = tmp_path / "core"
    (core / "configs/chips").mkdir(parents=True)
    (core / "docs/coverage").mkdir(parents=True)

    (core / "configs/chips").joinpath("stm32f103.yaml").write_text(
        yaml.safe_dump(
            {
                "name": "stm32f103",
                "arch": "arm",
                "core": "cortex-m3",
                "peripherals": [{"id": "usart1"}, {"id": "gpio"}],
            }
        )
    )
    (core / "configs/chips").joinpath("onboarding.yaml").write_text(
        yaml.safe_dump({"name": "onboarding", "arch": "arm"})
    )
    (core / "docs/coverage/chip-conformance.json").write_text(
        json.dumps([{"name": "stm32f103", "level": 1, "reg_match": 12, "excluded": 3}])
    )
    (core / "docs/coverage/tier1-matrix.json").write_text(
        json.dumps({"stm32f103": {"uart": {"status": "pass"}, "dma": {"status": "na"}}})
    )
    (core / "configs/chips-legacy.yaml").write_text(
        yaml.safe_dump({"excluded": {"pic16c63a": "Not Recommended for new designs."}})
    )

    priv = tmp_path / "ip/crates/labwired-ip-chips/chips"
    priv.mkdir(parents=True)
    # A private descriptor with internals that must NOT reach the output.
    (priv / "pic16f877a.yaml").write_text(
        yaml.safe_dump(
            {
                "name": "pic16f877a",
                "arch": "pic16",
                "core": "pic16",
                "flash": {"base": 0x0000, "size": "8KB"},
                "ram": {"base": 0x0020, "size": "368"},
                "package": {"name": "PDIP-40", "pin_count": 40},
                "peripherals": [{"id": "uart1"}],
            }
        )
    )
    (priv / "pic16c63a.yaml").write_text(
        yaml.safe_dump({"name": "pic16c63a", "arch": "pic16", "core": "pic16"})
    )
    tracker = tmp_path / "ip/parity-data/tracker.yaml"
    tracker.parent.mkdir(parents=True)
    tracker.write_text(
        yaml.safe_dump({"counted": {"pic16": ["pic16f877a"]}})
    )
    return core, priv, tracker


def test_private_descriptor_internals_never_leak(fake_tree):
    """The output may carry a part NAME, its package NAME and a gate status —
    never register maps, addresses, memory geometry, pin counts or comments."""
    core, priv, tracker = fake_tree
    data = catalog.build(core, priv, tracker)
    blob = json.dumps(data)
    for secret in ("8KB", "0x20", "368", "0x0000", "pin_count", "flash", "ram"):
        assert secret not in blob, f"private descriptor detail leaked: {secret}"


def test_private_part_shows_name_and_gate_status(fake_tree):
    core, priv, tracker = fake_tree
    data = catalog.build(core, priv, tracker)
    priv_rows = {c["name"]: c for c in data["chips"] if c["provenance"] == "private"}
    assert priv_rows["pic16f877a"]["coverage"] == "verified (P0-P5)"
    assert priv_rows["pic16f877a"]["parity_counted"] is True
    assert priv_rows["pic16f877a"]["package"] == "PDIP-40"


def test_retired_part_renders_excluded_not_supported(fake_tree):
    core, priv, tracker = fake_tree
    data = catalog.build(core, priv, tracker)
    rows = {c["name"]: c for c in data["chips"]}
    assert rows["pic16c63a"]["status"] == "excluded"
    assert rows["pic16c63a"]["excluded_reason"]
    assert rows["pic16f877a"]["status"] == "supported"


def test_public_part_keeps_conformance_level(fake_tree):
    core, priv, tracker = fake_tree
    data = catalog.build(core, priv, tracker)
    rows = {c["name"]: c for c in data["chips"]}
    assert rows["stm32f103"]["conformance_level"] == 1
    assert rows["stm32f103"]["coverage"] == "L1 modelled"
    assert rows["stm32f103"]["tier1"]["pass"] == 1
    assert rows["stm32f103"]["tier1"]["na"] == 1


def test_fixture_chips_are_excluded(fake_tree):
    core, priv, tracker = fake_tree
    data = catalog.build(core, priv, tracker)
    assert "onboarding" not in {c["name"] for c in data["chips"]}


def test_page_renders_every_chip(fake_tree):
    core, priv, tracker = fake_tree
    data = catalog.build(core, priv, tracker)
    md = page.render(data)
    for chip in data["chips"]:
        assert f'data-name="{chip["name"]}"' in md
    assert md.count('class="lw-chip"') == len(data["chips"])
    assert 'data-status="excluded"' in md


def test_public_snapshot_is_the_default(tmp_path, monkeypatch, fake_tree):
    """A default run writes the public catalogue even when the private one is
    present: publishing private names is an explicit opt-in."""
    core, priv, tracker = fake_tree
    out = core / "docs/coverage/chips.json"
    monkeypatch.setattr(
        sys, "argv",
        ["generate_chip_catalog.py", "--core-root", str(core), "--out", str(out),
         "--private-chips", str(priv), "--parity-tracker", str(tracker)],
    )
    assert catalog.main() == 0
    s = json.loads(out.read_text())["summary"]
    assert s["private"] == 0
    assert s["public"] == 1


def test_publish_private_opt_in(tmp_path, monkeypatch, fake_tree):
    core, priv, tracker = fake_tree
    out = core / "docs/coverage/chips.json"
    monkeypatch.setattr(
        sys, "argv",
        ["generate_chip_catalog.py", "--core-root", str(core), "--out", str(out),
         "--private-chips", str(priv), "--parity-tracker", str(tracker),
         "--publish-private"],
    )
    assert catalog.main() == 0
    s = json.loads(out.read_text())["summary"]
    assert s["private"] == 2
    assert s["public"] == 1


def test_require_private_fails_without_private(tmp_path, monkeypatch):
    core = tmp_path / "core"
    (core / "configs/chips").mkdir(parents=True)
    (core / "docs/coverage").mkdir(parents=True)
    monkeypatch.setattr(
        sys, "argv",
        ["generate_chip_catalog.py", "--core-root", str(core), "--require-private"],
    )
    assert catalog.main() == 2
