#!/usr/bin/env python3
"""List imported Renode chip descriptors whose size-less windows mask silicon.

A peripheral entry with no `size` gets a 4 KiB bus window (`push_peripheral`
in crates/core/src/bus/construct.rs). When the silicon block is smaller, the
window answers reads of the neighbouring addresses with zeros instead of a
bus fault, so firmware that touches an undeclared neighbour appears to work.

Routing gives a declared neighbour with a greater base priority, so the rows
listed here are proof that the 4 KiB default is larger than the block: the
gap between the base and the first declared neighbour is the upper bound of
the block size, and every undeclared address in the rest of the window is
masked.

Usage: /usr/bin/python3 scripts/audit_onboarding_window_sizes.py [--arch arm]
"""

import argparse
import glob
import os
import re
import sys

import yaml

DEFAULT_WINDOW = 0x1000
UNITS = {"": 1, "B": 1, "K": 1024, "KB": 1024, "KIB": 1024, "M": 1 << 20,
         "MIB": 1 << 20, "MB": 1_000_000, "GB": 1 << 30, "GIB": 1 << 30}


def parse_size(value):
    if value is None or isinstance(value, int):
        return value
    match = re.fullmatch(r"(0x[0-9a-fA-F]+|\d+)\s*([A-Za-z]*)", str(value).strip())
    return int(match.group(1), 0) * UNITS[match.group(2).upper()]


def parse_base(value):
    return int(value, 0) if isinstance(value, str) else value


def load(path):
    doc = yaml.safe_load(open(path)) or {}
    inherited = []
    includes = doc.get("include") or []
    for inc in includes if isinstance(includes, list) else [includes]:
        inherited += load(os.path.normpath(os.path.join(os.path.dirname(path), inc)))[1]
    own = doc.get("peripherals") or []
    ids = {p.get("id") for p in own}
    return doc, [p for p in inherited if p.get("id") not in ids] + own


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=os.path.join(os.path.dirname(__file__), ".."))
    parser.add_argument("--arch", help="only descriptors with this `arch`")
    args = parser.parse_args()

    rows = []
    for path in sorted(glob.glob(os.path.join(args.root, "configs/chips/onboarding/*.yaml"))):
        doc, periphs = load(path)
        if args.arch and doc.get("arch") != args.arch:
            continue
        windows = []
        for p in periphs:
            base = parse_base(p.get("base_address"))
            # Base 0 entries are board parts (LEDs, buttons, SPI devices), not MMIO.
            if base:
                windows.append((p["id"], base, parse_size(p.get("size"))))
        for pid, base, size in windows:
            if size is not None:
                continue
            covered = sorted((b, q) for q, b, _ in windows if q != pid and base < b < base + DEFAULT_WINDOW)
            if covered:
                rows.append((os.path.basename(path), pid, base, covered))

    for name, pid, base, covered in rows:
        neighbours = ", ".join(f"{q}@{b:#x}" for b, q in covered)
        print(f"{name}: {pid}@{base:#x} has no size; block <= {covered[0][0] - base:#x}; covers {neighbours}")
    print(f"{len(rows)} size-less windows cover a declared neighbour in {len({r[0] for r in rows})} descriptors")
    return 0


if __name__ == "__main__":
    sys.exit(main())
