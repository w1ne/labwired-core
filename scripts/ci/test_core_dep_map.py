"""Unit cover for scripts/ci/core_dep_map.py.

Parser and ratchet cases build small synthetic source trees. Two cases read the
real repo and assert only that the committed baseline and layers agree with the
committed source, which is the property the CI step enforces.
"""

import json
from pathlib import Path

import core_dep_map as dm
import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


def write_tree(root: Path, files: dict) -> Path:
    src = root / "src"
    for rel, text in files.items():
        p = src / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    return src


BASE_TREE = {
    "lib.rs": "pub mod bus;\npub mod peripherals;\npub mod cpu;\npub mod snapshot;\n",
    "bus.rs": "use crate::peripherals::esp32::Chip;\nfn f() { crate::peripherals::gpio::Port::new(); }\n",
    "snapshot.rs": "use crate::{bus::Bus, cpu::pic16::Snap};\n",
    "cpu/mod.rs": "pub mod pic16;\npub use pic16::Pic16;\n",
    "cpu/pic16.rs": "use crate::bus::Bus;\nuse super::Pic16 as X;\n",
    "peripherals/mod.rs": "pub mod esp32;\npub mod gpio;\n",
    "peripherals/esp32/mod.rs": "pub mod uart;\nuse crate::bus::Bus;\n",
    "peripherals/esp32/uart.rs": "use super::super::gpio::Port;\nuse crate::SimResult;\n",
    "peripherals/gpio/mod.rs": "pub struct Port;\n",
}

LAYERS = {
    "kernel": ["lib", "bus", "peripherals", "peripherals/gpio", "snapshot", "cpu"],
    "leaf": ["peripherals/esp32", "cpu/pic16"],
}


def test_expands_use_groups_inline_paths_and_super(tmp_path):
    g = dm.build_graph(write_tree(tmp_path, BASE_TREE))
    m = g["modules"]
    assert m["bus"]["imports"] == {"peripherals/esp32": 1, "peripherals/gpio": 1}
    assert m["snapshot"]["imports"] == {"bus": 1, "cpu/pic16": 1}
    # `super::super::gpio` from peripherals/esp32/uart.rs resolves to peripherals/gpio
    assert m["peripherals/esp32"]["imports"]["peripherals/gpio"] == 1
    # items defined in lib.rs map to the lib node
    assert m["peripherals/esp32"]["imports"]["lib"] == 1
    # cpu/mod.rs re-export of a child module is an edge to that child
    assert m["cpu"]["imports"] == {"cpu/pic16": 1}


def test_cfg_test_and_mod_tests_are_not_production(tmp_path):
    tree = dict(BASE_TREE)
    tree["bus.rs"] = (
        "use crate::snapshot::S;\n"
        "fn prod() {}\n"
        "#[cfg(test)]\n"
        "mod unit {\n"
        "    use crate::peripherals::esp32::T;\n"
        "    fn t() { let _ = \"}\"; }\n"
        "}\n"
        "mod tests {\n"
        "    use crate::cpu::pic16::T;\n"
        "}\n"
        "#[cfg(test)]\nuse crate::peripherals::esp32::Only;\n"
        "fn after() {}\n"
    )
    tree["bus/tests_main.rs"] = "use crate::peripherals::esp32::X;\n"
    tree["bus_tests.rs"] = "use crate::peripherals::esp32::X;\n"
    g = dm.build_graph(write_tree(tmp_path, tree))
    bus = g["modules"]["bus"]
    assert bus["imports"] == {"snapshot": 1}
    assert bus["production_lines"] == 3  # use, fn prod, fn after
    assert bus["test_lines"] > 5


def test_cfg_test_mod_declaration_excludes_its_file(tmp_path):
    tree = dict(BASE_TREE)
    tree["bus.rs"] = "#[cfg(test)]\nmod helpers;\nfn a() {}\n"
    tree["bus/helpers.rs"] = "use crate::peripherals::esp32::X;\nfn h() {}\n"
    g = dm.build_graph(write_tree(tmp_path, tree))
    assert g["modules"]["bus"]["imports"] == {}
    assert g["modules"]["bus"]["production_lines"] == 3 - 2


def test_comments_and_strings_are_ignored(tmp_path):
    tree = dict(BASE_TREE)
    tree["bus.rs"] = (
        '// crate::peripherals::esp32::A\n'
        '/* crate::peripherals::esp32::B /* nested */ crate::peripherals::esp32::C */\n'
        'const S: &str = "crate::peripherals::esp32::D";\n'
        'const R: &str = r#"crate::peripherals::esp32::E " still"#;\n'
        "const Q: char = '\"';\n"
        "use crate::snapshot::Real;\n"
    )
    g = dm.build_graph(write_tree(tmp_path, tree))
    assert g["modules"]["bus"]["imports"] == {"snapshot": 1}


def test_output_is_deterministic(tmp_path):
    src = write_tree(tmp_path, BASE_TREE)
    a = json.dumps(dm.build_graph(src), sort_keys=True)
    b = json.dumps(dm.build_graph(src), sort_keys=True)
    assert a == b


def test_scc_reported(tmp_path):
    g = dm.build_graph(write_tree(tmp_path, BASE_TREE))
    # bus -> esp32 -> bus is the only cycle in the fixture
    assert g["sccs"] == [["bus", "peripherals/esp32"]]
    assert g["summary"]["largest_scc_modules"] == len(g["sccs"][0])


def _setup(tmp_path, tree=None):
    src = write_tree(tmp_path, tree or BASE_TREE)
    layers = tmp_path / "layers.json"
    layers.write_text(json.dumps(LAYERS))
    return src, layers, tmp_path / "baseline.json"


def _run(src, layers, baseline, *extra):
    return dm.main(["--src", str(src), "--layers", str(layers), "--baseline", str(baseline), *extra])


def test_ratchet_passes_at_baseline_and_fails_on_new_edge(tmp_path, capsys):
    src, layers, baseline = _setup(tmp_path)
    assert _run(src, layers, baseline, "--update-baseline") == 0
    kb = json.loads(baseline.read_text())["kernel_to_leaf"]
    assert kb["bus -> peripherals/esp32"] == 1
    assert kb["snapshot -> cpu/pic16"] == 1
    assert _run(src, layers, baseline, "--check") == 0
    # a kernel module gains a dependency on a leaf it did not use before
    (src / "peripherals/gpio/mod.rs").write_text("use crate::cpu::pic16::X;\npub struct Port;\n")
    capsys.readouterr()
    assert _run(src, layers, baseline, "--check", "--strict") == 1
    out = capsys.readouterr().out
    assert "peripherals/gpio -> cpu/pic16" in out
    assert "::warning::new kernel->leaf dependency" in out
    # report mode never fails on increases
    assert _run(src, layers, baseline, "--check") == 0


def test_report_mode_writes_step_summary(tmp_path, monkeypatch):
    src, layers, baseline = _setup(tmp_path)
    _run(src, layers, baseline, "--update-baseline")
    p = src / "bus.rs"
    p.write_text(p.read_text() + "use crate::peripherals::esp32::More;\n")
    summ = tmp_path / "summary.md"
    monkeypatch.setenv("GITHUB_STEP_SUMMARY", str(summ))
    assert _run(src, layers, baseline, "--check") == 0
    text = summ.read_text()
    assert "advisory" in text and "grew from 1 to 2" in text


def test_unclassified_module_warns_in_report_and_fails_in_strict(tmp_path, capsys):
    tree = dict(BASE_TREE)
    tree["newmod.rs"] = "fn f() {}\n"
    tree["lib.rs"] += "pub mod newmod;\n"
    src, layers, baseline = _setup(tmp_path, tree)
    _run(src, layers, baseline, "--update-baseline")
    capsys.readouterr()
    assert _run(src, layers, baseline, "--check") == 0
    assert "::warning::core_layers.json does not classify: newmod" in capsys.readouterr().out
    assert _run(src, layers, baseline, "--check", "--strict") == 1


def test_unparsable_layers_fail_in_report_mode(tmp_path):
    src, layers, baseline = _setup(tmp_path)
    _run(src, layers, baseline, "--update-baseline")
    layers.write_text("{not json")
    with pytest.raises(ValueError):
        _run(src, layers, baseline, "--check")


def test_ratchet_fails_when_existing_edge_count_grows(tmp_path, capsys):
    src, layers, baseline = _setup(tmp_path)
    _run(src, layers, baseline, "--update-baseline")
    p = src / "bus.rs"
    p.write_text(p.read_text() + "use crate::peripherals::esp32::More;\n")
    assert _run(src, layers, baseline, "--check", "--strict") == 1
    assert "grew from 1 to 2" in capsys.readouterr().out


def test_ratchet_does_not_track_leaf_to_kernel_or_neutral(tmp_path):
    tree = dict(BASE_TREE)
    tree["neutral.rs"] = "use crate::peripherals::esp32::X;\n"
    tree["lib.rs"] += "pub mod neutral;\n"
    src, layers, baseline = _setup(tmp_path, tree)
    _run(src, layers, baseline, "--update-baseline")
    assert not any(k.startswith("neutral") for k in json.loads(baseline.read_text())["kernel_to_leaf"])
    assert _run(src, layers, baseline, "--check") == 0


def test_decrease_passes_and_asks_for_baseline_update(tmp_path, capsys):
    src, layers, baseline = _setup(tmp_path)
    _run(src, layers, baseline, "--update-baseline")
    (src / "bus.rs").write_text("fn f() {}\n")
    capsys.readouterr()
    assert _run(src, layers, baseline, "--check") == 0
    assert "lower the baseline" in capsys.readouterr().out
    assert _run(src, layers, baseline, "--update-baseline") == 0
    assert "bus -> peripherals/esp32" not in json.loads(baseline.read_text())["kernel_to_leaf"]


def test_update_baseline_refuses_increase_without_flag(tmp_path, capsys):
    src, layers, baseline = _setup(tmp_path)
    _run(src, layers, baseline, "--update-baseline")
    before = baseline.read_text()
    p = src / "bus.rs"
    p.write_text(p.read_text() + "use crate::peripherals::esp32::More;\n")
    assert _run(src, layers, baseline, "--update-baseline") == 1
    assert baseline.read_text() == before
    assert _run(src, layers, baseline, "--update-baseline", "--allow-increase") == 0
    assert json.loads(baseline.read_text())["kernel_to_leaf"]["bus -> peripherals/esp32"] == 2


def test_node_in_both_sets_is_rejected(tmp_path):
    src, layers, baseline = _setup(tmp_path)
    layers.write_text(json.dumps({"kernel": ["bus"], "leaf": ["bus"]}))
    with pytest.raises(SystemExit):
        _run(src, layers, baseline, "--check")


@pytest.fixture(scope="module")
def real_graph():
    return dm.build_graph(dm.DEFAULT_SRC)


def test_committed_baseline_parses_and_report_mode_runs_clean(real_graph, capsys):
    kernel, leaf = dm.load_layers(dm.DEFAULT_LAYERS)
    base = json.loads(dm.DEFAULT_BASELINE.read_text())["kernel_to_leaf"]
    assert base and all(isinstance(v, int) and v > 0 for v in base.values())
    # advisory: must succeed whatever state main is in
    assert dm.run_check(real_graph, kernel, leaf, dm.DEFAULT_BASELINE, neutral=dm.load_neutral(dm.DEFAULT_LAYERS)) == 0
    assert "largest strongly connected component" in capsys.readouterr().out


def test_committed_layers_parse_without_overlap():
    kernel, leaf = dm.load_layers(dm.DEFAULT_LAYERS)
    assert kernel and leaf
