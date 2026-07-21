import json
import subprocess
import sys
from pathlib import Path

import board_matrix as bm

REPO_ROOT = Path(__file__).resolve().parents[2]


def _entry(**over):
    base = dict(
        id="demo-board",
        kind="firmware-gate",
        path="examples/iolink-station",
        apt=["gcc-arm-none-eabi"],
        rust_targets=[],
        packs=["stm32cubel4@v1.18.2"],
        submodules="recursive",
        gate=True,
    )
    base.update(over)
    return base


def test_select_pull_request_keeps_only_gates():
    entries = [_entry(id="a", gate=True), _entry(id="b", gate=False)]
    assert [e["id"] for e in bm.select(entries, "pull_request")] == ["a"]


def test_select_schedule_keeps_all():
    entries = [_entry(id="a", gate=True), _entry(id="b", gate=False)]
    assert [e["id"] for e in bm.select(entries, "schedule")] == ["a", "b"]


def test_validate_flags_missing_build_script():
    bad = _entry(path="examples/does-not-exist")
    errors = bm.validate([bad], str(REPO_ROOT))
    assert any("ci/build.sh" in e for e in errors)


def test_validate_passes_for_real_iolink_entry():
    entries = bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))
    assert bm.validate(entries, str(REPO_ROOT)) == []


def test_to_matrix_joins_lists_to_strings():
    m = bm.to_matrix([_entry(apt=["gcc-arm-none-eabi"], rust_targets=["thumbv6m-none-eabi"], packs=["stm32cubel4@v1.18.2"])])
    inc = m["include"][0]
    assert inc["apt"] == "gcc-arm-none-eabi"
    assert inc["rust_targets"] == "thumbv6m-none-eabi"
    assert inc["packs"] == "stm32cubel4@v1.18.2"
    assert "toolchains" not in inc


def test_cli_emits_json_for_pull_request():
    out = subprocess.check_output(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT)],
        text=True,
    )
    matrix = json.loads(out)
    assert any(e["id"] == "iolink-station-l476" for e in matrix["include"])


def test_iolink_station_installs_newlib_for_nano_specs():
    entries = bm.load_manifest(str(REPO_ROOT / "configs/ci/boards.yml"))
    iolink = next(e for e in entries if e["id"] == "iolink-station-l476")
    matrix = bm.to_matrix([iolink])
    apt = matrix["include"][0]["apt"].split()
    assert "gcc-arm-none-eabi" in apt
    assert "libnewlib-arm-none-eabi" in apt


def test_cli_exits_nonzero_on_invalid_manifest(tmp_path):
    bad = tmp_path / "boards.yml"
    bad.write_text(
        "boards:\n"
        "  - id: ghost\n"
        "    kind: firmware-gate\n"
        "    path: examples/does-not-exist\n"
        "    gate: true\n"
    )
    result = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT),
         "--manifest", str(bad)],
        capture_output=True, text=True,
    )
    assert result.returncode != 0
    assert "ci/build.sh" in result.stderr


def test_cli_exits_nonzero_on_empty_manifest(tmp_path):
    empty = tmp_path / "boards.yml"
    empty.write_text("boards: []\n")
    result = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/ci/board_matrix.py"),
         "--event", "pull_request", "--repo-root", str(REPO_ROOT),
         "--manifest", str(empty)],
        capture_output=True, text=True,
    )
    assert result.returncode != 0
    assert "no boards" in result.stderr
