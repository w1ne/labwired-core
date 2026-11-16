# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Load a chip descriptor the way the simulator does, `include:` expanded.

A chip YAML may `include:` a base descriptor (a package variant of one die,
e.g. configs/chips/rp2354b.yaml on rp2350.yaml) and overlay only what differs.
The Rust loader (crates/config/src/chip_merge.rs) merges: includes first, local
keys win, `peripherals` replace by `id`, `memory_regions` by `name`, `pins` and
`analog_pins` merge by label, every other key is replaced whole. A script that
read the raw file would see a variant with no arch, RAM or base peripherals,
so every script that reads descriptor fields loads them through here.

`merge_chip` is plain-dict code with no YAML dependency, so a script with its
own parser (generate_compat_matrix.py) can reuse it.
"""

from __future__ import annotations

from pathlib import Path
from typing import Callable


def _merge_seq_by(base: list | None, overlay: list | None, field: str) -> list:
    items = list(base or [])
    for item in overlay or []:
        key = item.get(field) if isinstance(item, dict) else None
        if key is not None:
            for i, existing in enumerate(items):
                if isinstance(existing, dict) and existing.get(field) == key:
                    items[i] = item
                    break
            else:
                items.append(item)
        else:
            items.append(item)
    return items


def merge_chip(base: dict, overlay: dict) -> dict:
    """Overlay one chip mapping on another, as `merge_chip_yaml` does."""
    out = dict(base)
    for key, value in overlay.items():
        if key == "include":
            continue
        if key == "peripherals":
            out[key] = _merge_seq_by(out.get(key), value, "id")
        elif key == "memory_regions":
            out[key] = _merge_seq_by(out.get(key), value, "name")
        elif key in ("pins", "analog_pins"):
            merged = dict(out.get(key) or {})
            merged.update(value or {})
            out[key] = merged
        else:
            out[key] = value
    return out


def _includes(doc: dict) -> list[str]:
    inc = doc.get("include")
    if inc is None:
        return []
    if isinstance(inc, str):
        return [inc]
    if isinstance(inc, list) and all(isinstance(i, str) for i in inc):
        return list(inc)
    raise ValueError("include must be a string or a list of strings")


def load_chip(
    path: Path, read: Callable[[Path], dict] | None = None, _stack: tuple = ()
) -> dict:
    """The descriptor at `path` with its includes expanded (relative paths).

    `read` parses one file into a dict; PyYAML's safe_load by default.
    """
    if read is None:
        import yaml

        def read(p: Path) -> dict:
            return yaml.safe_load(p.read_text()) or {}

    path = Path(path)
    canon = path.resolve()
    if canon in _stack:
        raise ValueError(f"cycle detected in chip YAML include of {path}")
    doc = read(path) or {}
    merged: dict = {}
    for rel in _includes(doc):
        inc = path.parent / rel
        if not inc.is_file():
            raise FileNotFoundError(f"chip YAML include not found: {inc}")
        merged = merge_chip(merged, load_chip(inc, read, _stack + (canon,)))
    return merge_chip(merged, doc) if merged else doc
