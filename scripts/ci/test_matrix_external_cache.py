"""The Arduino matrix must not serve a cached ELF for an external-compile cell.

`compile_fingerprint` hashes the sketch sources and the PlatformIO
platform/board/framework strings. Those strings pin the toolchain for a
PlatformIO cell, so its digest describes every input to the ELF. They pin
nothing for a cell that declares `external_compile:`, whose compiler is a
driver in another repository — for brd2709a, services/labwired-builder/
silabs-arduino in the monorepo.

So editing that compiler changes the ELF a user gets and leaves the digest
identical. Measured 2026-08-24: the brd2709a lane last really compiled on
2026-08-22 and the five runs after it were 8/8 "compile: cache hit", 0
compiles, done in 1s — two of them triggered BY a change to that compiler,
which is the one case the lane exists to catch. Five green columns proving
nothing.

Wired into pr-gate's pytest line in core-ci.yml.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

CORE_ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(CORE_ROOT / "validation"))

from matrix_lib import cacheable_cell  # noqa: E402
from matrix_lib.cache import compile_fingerprint, elf_cache_hit, write_fingerprint  # noqa: E402

RUN_MATRIX = CORE_ROOT / "validation" / "arduino-matrix" / "run_matrix.py"


def test_a_platformio_cell_stays_cacheable() -> None:
    # The cache exists for the 17-board PlatformIO fleet; this must not disable it.
    assert cacheable_cell(None) is True


def test_an_external_compile_cell_is_not_cacheable() -> None:
    assert cacheable_cell({"target": "silabs-arduino:xg26explorerkit"}) is False


def test_the_rule_ignores_the_driver_target_and_looks_only_at_presence() -> None:
    # Any external driver is out of the digest's reach, not just the silabs one.
    assert cacheable_cell({}) is False
    assert cacheable_cell({"target": "anything-at-all"}) is False


def test_the_digest_really_cannot_see_the_external_compiler(tmp_path: Path) -> None:
    """The defect itself, reproduced.

    Two fingerprints taken across a change to the COMPILER — which lives
    outside `sketch_src` — come out identical, and `elf_cache_hit` says hit.
    That is why `cacheable_cell` has to refuse rather than the digest catching it.
    """
    sketch = tmp_path / "src"
    sketch.mkdir()
    (sketch / "main.ino").write_text("void setup(){} void loop(){}", encoding="utf-8")

    compiler = tmp_path / "silabs-arduino"
    compiler.mkdir()
    (compiler / "variant.h").write_text("#define LED 8", encoding="utf-8")

    def digest() -> str:
        return compile_fingerprint(
            board_id="brd2709a",
            sketch_id="L2_blink_serial",
            sketch_src=sketch,
            pio_platform="external",
            pio_board="silabs-arduino:xg26explorerkit",
            pio_framework="arduino",
            extra={"max_steps": 1000, "led_watch": None},
        )

    before = digest()
    (compiler / "variant.h").write_text("#define LED 9", encoding="utf-8")
    after = digest()

    assert before == after, "if this ever differs, the digest grew reach and the rule can relax"

    cell = tmp_path / "cell"
    cell.mkdir()
    (cell / "firmware.elf").write_bytes(b"\x7fELF stale")
    write_fingerprint(cell, before)
    assert elf_cache_hit(cell, after) is True, "the stale ELF is served — this is the defect"


def test_run_matrix_actually_consults_the_rule() -> None:
    """A predicate nothing calls is not a fix.

    Asserts the cache-hit branch in run_matrix.py is guarded by `cacheable`,
    and that `cacheable` comes from the shared helper rather than a second
    copy of the rule that can drift from it.
    """
    source = RUN_MATRIX.read_text(encoding="utf-8")

    assert "cacheable = cacheable_cell(ext)" in source, "run_matrix.py restates or drops the rule"

    hit_branches = [
        line
        for line in source.splitlines()
        if "elf_cache_hit(" in line and not line.lstrip().startswith("#")
    ]
    assert hit_branches, "no elf_cache_hit call found — did the cache move?"

    serving = [line for line in hit_branches if "not cacheable" not in line]
    assert serving, "expected a branch that serves the cache"
    for line in serving:
        assert "cacheable" in line, f"a cache hit is served without checking cacheable: {line.strip()}"


def test_the_skipped_compile_is_announced() -> None:
    """A cell that recompiles despite a matching digest must say why.

    Otherwise the lane silently spends the time and nobody can tell a
    non-cacheable cell from a cache that is simply cold.
    """
    source = RUN_MATRIX.read_text(encoding="utf-8")
    assert re.search(r"not cacheable", source), "the non-cacheable path prints nothing"
