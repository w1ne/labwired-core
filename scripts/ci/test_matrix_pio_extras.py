"""Per-board PlatformIO extras in boards.yaml must reach the build and the cache.

The CH32V003 row needs two: `build_flags` names a board LED (WCH's
CH32V003F4 variant leaves LED_BUILTIN as PNUM_NOT_DEFINED, and
digitalWrite() on it compiles to a trap), and `platform_packages` pins a
framework that the platform names only by git branch. Both change the ELF, so
both must change the compile digest — otherwise a flag edit is served the ELF
built without it.

Wired into pr-gate's pytest line in core-ci.yml.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

CORE_ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(CORE_ROOT / "validation"))

from matrix_lib.cache import compile_fingerprint  # noqa: E402

_spec = importlib.util.spec_from_file_location(
    "run_matrix", CORE_ROOT / "validation" / "arduino-matrix" / "run_matrix.py"
)
run_matrix = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(run_matrix)

SKETCH = CORE_ROOT / "validation" / "arduino-matrix" / "sketches" / "L2_blink_serial" / "src"


def _board(**pio_extra) -> dict:
    return {"pio": {"platform": "p", "board": "b", "framework": "arduino", **pio_extra}}


def test_extras_land_in_platformio_ini(tmp_path) -> None:
    board = _board(
        build_flags=["-DLED_BUILTIN=PD0"],
        platform_packages=["framework-x @ https://example.invalid/x.git#abc"],
    )
    ini = (run_matrix.write_pio_project(tmp_path, board, SKETCH) / "platformio.ini").read_text()
    assert "    -DCORE_DEBUG_LEVEL=0\n    -DLED_BUILTIN=PD0\n" in ini
    assert "platform_packages =\n    framework-x @ https://example.invalid/x.git#abc\n" in ini


def test_no_extras_writes_the_old_ini(tmp_path) -> None:
    ini = (run_matrix.write_pio_project(tmp_path, _board(), SKETCH) / "platformio.ini").read_text()
    assert ini.rstrip().endswith("-DCORE_DEBUG_LEVEL=0")
    assert "platform_packages" not in ini


def test_ch32v003_row_names_its_led_and_pins_its_framework() -> None:
    boards = {b["id"]: b for b in run_matrix.load_config()["boards"]}
    pio = boards["ch32v003"]["pio"]
    assert "-DLED_BUILTIN=PD0" in pio["build_flags"]
    assert boards["ch32v003"]["led_watch"] == "gpiod:0"
    # A pin is a full commit sha, never a branch.
    assert len(pio["platform"].split("#")[1]) == 40
    assert all(len(p.split("#")[1]) == 40 for p in pio["platform_packages"])


def _digest(extra: dict) -> str:
    return compile_fingerprint(
        board_id="ch32v003",
        sketch_id="L2_blink_serial",
        sketch_src=SKETCH,
        pio_platform="p",
        pio_board="b",
        pio_framework="arduino",
        extra=extra,
    )


def test_a_build_flag_edit_changes_the_digest() -> None:
    base = {"max_steps": 1, "led_watch": "gpiod:0"}
    assert _digest({**base, "build_flags": ["-DLED_BUILTIN=PD0"]}) != _digest(
        {**base, "build_flags": ["-DLED_BUILTIN=PD1"]}
    )
