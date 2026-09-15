"""Content-hash helpers so matrix cells can skip recompile when unchanged."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any


def _hash_tree(h: Any, root: Path) -> None:  # h: hashlib hash object
    if not root.is_dir():
        return
    for p in sorted(root.rglob("*")):
        if p.is_file():
            rel = p.relative_to(root).as_posix()
            h.update(b"PATH\0")
            h.update(rel.encode())
            h.update(p.read_bytes())


def compile_fingerprint(
    *,
    board_id: str,
    sketch_id: str,
    sketch_src: Path,
    pio_platform: str,
    pio_board: str,
    pio_framework: str,
    extra: dict[str, Any] | None = None,
) -> str:
    """Stable hex digest of inputs that affect the compiled ELF."""
    h = hashlib.sha256()
    h.update(b"v1\0")
    for part in (board_id, sketch_id, pio_platform, pio_board, pio_framework):
        h.update(part.encode())
        h.update(b"\0")
    _hash_tree(h, sketch_src)
    if extra:
        h.update(json.dumps(extra, sort_keys=True, default=str).encode())
    return h.hexdigest()


def cacheable_cell(external_compile: dict[str, Any] | None) -> bool:
    """False when the cell's compiler lies outside the fingerprint's reach.

    ``compile_fingerprint`` hashes the sketch sources and the PlatformIO
    platform/board/framework strings. For a PlatformIO cell those strings pin
    the toolchain, so the digest describes every input to the ELF.

    A cell that declares ``external_compile:`` is compiled by a driver in
    ANOTHER repository, and not one byte of that driver reaches the digest — so
    editing it changes the ELF and leaves the digest identical, and the cell
    would hit a cache built from the previous compiler. Measured 2026-08-24 on
    the brd2709a lane: five consecutive green runs, 8/8 "cache hit", zero
    compiles, two of them triggered BY a change to that compiler.

    Fingerprinting the driver would mean reaching into a repo this one does not
    own and guessing which of its files matter. Refusing to cache is the honest
    answer: a cache that cannot see its own compiler must not claim a hit.
    """
    return external_compile is None


def fingerprint_path(cell_out: Path) -> Path:
    return cell_out / ".compile_fingerprint"


def read_fingerprint(cell_out: Path) -> str | None:
    p = fingerprint_path(cell_out)
    if not p.is_file():
        return None
    return p.read_text(encoding="utf-8").strip() or None


def write_fingerprint(cell_out: Path, digest: str) -> None:
    fingerprint_path(cell_out).write_text(digest + "\n", encoding="utf-8")


def elf_cache_hit(cell_out: Path, digest: str) -> bool:
    elf = cell_out / "firmware.elf"
    return elf.is_file() and read_fingerprint(cell_out) == digest
