#!/usr/bin/env python3
"""Merge the public and private chip catalogues into one coverage JSON.

Public side (this repo):
  configs/chips/*.yaml            one descriptor per modelled part
  docs/coverage/chip-conformance.json   conformance level L0-L2 + reg match
  docs/coverage/tier1-matrix.json       per-chip x subsystem pass/na/unrecorded

Private side (w1ne/labwired-ip, descriptors are NOT public):
  crates/labwired-ip-chips/chips/*.yaml  one private descriptor per part
  parity-data/tracker.yaml `counted:`    parts with full P0-P5 evidence
  parity-data/tracker.yaml `excluded:`   retired parts (not buyable / legacy)

Legacy parts: a part is listed under the tracker's `excluded:` block once it is
dropped (obsolete, not buyable, or replacd). The page renders it as "excluded"
instead of "supported" so a retired part is never shown as covered, and it stays
visible in the table so a reader sees WHY a part they expected is gone. Core
also carries a small `configs/chips-legacy.yaml` for parts dropped before the
tracker grew an `excluded:` block; both lists are merged.

The private catalogue is a private repository, so a docs build may not have it.
The committed snapshot (docs/coverage/chips.json) is the PUBLIC catalogue by
default; pass --publish-private to include the private model names, which is an
explicit choice because this repo and docs.labwired.com are public and the
private catalogue is marked NOT FOR PUBLICATION.

Only names + coverage status leave this script: private descriptor contents
(register maps, base addresses, flash/ram geometry, comments) are read to
COUNT them and are never written to the output.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.stderr.write("PyYAML is required: pip install pyyaml\n")
    raise

# Chip-id stems that are test fixtures or scratch parts, not product chips:
# they are excluded from the published catalogue so the page lists real parts.
EXCLUDED_STEMS = {"onboarding"}

# Vendor is not a field in the descriptors, so it is derived from the part-name
# prefix. Longest prefix wins. Families are the arch/core pair the descriptor
# already carries; these two maps are display metadata only.
VENDORS = (
    ("pic12", "Microchip"),
    ("pic16", "Microchip"),
    ("pic18", "Microchip"),
    ("pic24", "Microchip"),
    ("dspic", "Microchip"),
    ("atmega", "Microchip"),
    ("attiny", "Microchip"),
    ("at90usb", "Microchip"),
    ("atsamd", "Microchip"),
    ("sam", "Microchip"),
    ("esp32", "Espressif"),
    ("stm32", "STMicroelectronics"),
    ("nrf", "Nordic Semiconductor"),
    ("rp20", "Raspberry Pi"),
    ("rp23", "Raspberry Pi"),
    ("k32", "NXP"),
    ("mkw", "NXP"),
    ("imxrt", "NXP"),
    ("ra4", "Renesas"),
    ("ra6", "Renesas"),
    ("ra8", "Renesas"),
    ("efr32", "Silicon Labs"),
    ("cc25", "Texas Instruments"),
    ("rv1103", "Rockchip"),
    ("litex", "LiteX"),
    ("arty", "Digilent"),
    ("vega", "OpenHW"),
    ("apollo", "Ambiq"),
)

ARCH_FAMILY = {
    "pic16": "PIC16",
    "pic18": "PIC18",
    "pic24": "PIC24 / dsPIC33",
    "avr": "AVR",
    "arm": "Arm Cortex-M",
    "arm-cortex-m": "Arm Cortex-M",
    "xtensa-lx6": "Xtensa LX6",
    "xtensa-lx7": "Xtensa LX7",
    "riscv": "RISC-V",
    "riscv32": "RISC-V",
    "unknown": "Other",
    "": "Other",
}


def vendor_of(name: str) -> str:
    low = name.lower()
    for prefix, vendor in VENDORS:
        if low.startswith(prefix):
            return vendor
    return "Other"


def family_of(arch: str, name: str) -> str:
    if arch in ARCH_FAMILY and arch != "unknown":
        return ARCH_FAMILY[arch]
    # Fall back to the naming family so an unknown arch is not a black hole.
    low = name.lower()
    if low.startswith("pic"):
        return low[:5].upper()
    return ARCH_FAMILY.get(arch, "Other")


def load_yaml(path: Path) -> dict:
    with path.open() as fh:
        return yaml.safe_load(fh) or {}


def load_json(path: Path, default):
    if not path.exists():
        return default
    with path.open() as fh:
        return json.load(fh)


def package_of(desc: dict) -> str:
    """Package label from a descriptor, or '' when it carries none.

    Reads only the `package` block, which is board-level metadata. No private
    geometry (pin count, body size, base addresses) is copied out.
    """
    pkg = desc.get("package")
    if isinstance(pkg, dict):
        return str(pkg.get("name") or "").strip()
    if isinstance(pkg, str):
        return pkg.strip()
    variants = desc.get("package_variants")
    if isinstance(variants, list) and variants:
        names = []
        for v in variants:
            if isinstance(v, dict) and v.get("name"):
                names.append(str(v["name"]).strip())
            elif isinstance(v, str):
                names.append(v.strip())
        if names:
            return " / ".join(sorted(set(names)))
    return ""


def coverage_label(level: int | None, counted: bool, public: bool) -> str:
    """One plain word a visitor can sort on."""
    if public:
        if level is None:
            return "modelled"
        return {0: "L0 estate", 1: "L1 modelled", 2: "L2 verified"}.get(level, f"L{level}")
    # Private: the published status is the parity gate.
    return "verified (P0-P5)" if counted else "modelled (private)"


def tier1_summary(cells: dict | None) -> dict:
    """Roll the per-subsystem matrix up to counts. No per-cell detail leaks."""
    out = {"pass": 0, "partial": 0, "blocked": 0, "na": 0, "unrecorded": 0}
    if not cells:
        return out
    for cell in cells.values():
        status = (cell or {}).get("status", "unrecorded")
        if status not in out:
            status = "unrecorded"
        out[status] += 1
    return out


def load_legacy(core_root: Path, tracker: Path | None) -> dict[str, str]:
    """Parts that must render as excluded, mapped to a one-line reason.

    Two sources, merged: the private tracker's `excluded:` block (a mapping of
    part -> reason in the current schema, or a list of parts in the older one)
    and core's committed `configs/chips-legacy.yaml`. Nothing else about the
    private tracker is read.
    """
    legacy: dict[str, str] = {}

    if tracker and tracker.exists():
        block = load_yaml(tracker).get("excluded")
        if isinstance(block, dict):
            for part, reason in block.items():
                legacy[str(part).strip().lower()] = str(reason or "retired").strip()
        elif isinstance(block, list):
            for part in block:
                legacy[str(part).strip().lower()] = "retired"

    core_legacy = core_root / "configs/chips-legacy.yaml"
    if core_legacy.exists():
        block = load_yaml(core_legacy) or {}
        for part, reason in (block.get("excluded") or {}).items():
            legacy.setdefault(str(part).strip().lower(), str(reason or "retired").strip())

    return legacy


def build(
    core_root: Path,
    private_chips: Path | None,
    parity_tracker: Path | None,
) -> dict:
    conf_rows = load_json(core_root / "docs/coverage/chip-conformance.json", [])
    conf = {r["name"]: r for r in conf_rows if isinstance(r, dict) and r.get("name")}
    tier1 = load_json(core_root / "docs/coverage/tier1-matrix.json", {})
    legacy = load_legacy(core_root, parity_tracker)

    chips: list[dict] = []
    public_ids: set[str] = set()

    for path in sorted((core_root / "configs/chips").glob("*.yaml")):
        stem = path.stem
        if stem in EXCLUDED_STEMS:
            continue
        desc = load_yaml(path)
        name = str(desc.get("name") or stem)
        arch = str(desc.get("arch") or "")
        row = conf.get(name)
        level = row.get("level") if row else None
        peripherals = sum(
            1
            for p in (desc.get("peripherals") or [])
            if isinstance(p, dict)
        )
        public_ids.add(name)
        reason = legacy.get(name.lower())
        chips.append(
            {
                "name": name,
                "vendor": vendor_of(name),
                "family": family_of(arch, name),
                "arch": arch or "unknown",
                "core": str(desc.get("core") or ""),
                "package": package_of(desc),
                "provenance": "public",
                "coverage": coverage_label(level, False, True),
                "conformance_level": level,
                "reg_match": (row or {}).get("reg_match"),
                "reg_match_excluded": (row or {}).get("excluded"),
                "peripherals": peripherals,
                "tier1": tier1_summary(tier1.get(name)),
                "parity_counted": None,
                "status": "excluded" if reason else "supported",
                "excluded_reason": reason or None,
            }
        )

    private_count = 0
    if private_chips and private_chips.is_dir():
        counted: set[str] = set()
        if parity_tracker and parity_tracker.exists():
            tracker = load_yaml(parity_tracker)
            for family in (tracker.get("counted") or {}).values():
                for part in family or []:
                    counted.add(str(part).strip().lower())
        for path in sorted(private_chips.glob("*.yaml")):
            stem = path.stem
            if stem in EXCLUDED_STEMS:
                continue
            desc = load_yaml(path)
            name = str(desc.get("name") or stem)
            # A private descriptor for a part the public catalogue already
            # models is the same chip: keep the public row, note the alias.
            if name in public_ids:
                continue
            arch = str(desc.get("arch") or "")
            is_counted = name.lower() in counted
            reason = legacy.get(name.lower())
            private_count += 1
            chips.append(
                {
                    "name": name,
                    "vendor": vendor_of(name),
                    "family": family_of(arch, name),
                    "arch": arch or "unknown",
                    "core": str(desc.get("core") or ""),
                    "package": package_of(desc),
                    "provenance": "private",
                    "coverage": coverage_label(None, is_counted, False),
                    "conformance_level": None,
                    "reg_match": None,
                    "reg_match_excluded": None,
                    "peripherals": len(
                        [p for p in (desc.get("peripherals") or []) if isinstance(p, dict)]
                    ),
                    "tier1": tier1_summary(None),
                    "parity_counted": is_counted,
                    "status": "excluded" if reason else "supported",
                    "excluded_reason": reason or None,
                }
            )
    elif private_chips:
        print(
            f"notice: private chip dir not found at {private_chips}; "
            "the page will list public chips only",
            file=sys.stderr,
        )

    chips.sort(key=lambda c: (c["vendor"], c["family"], c["name"]))

    by_vendor: dict[str, int] = {}
    by_family: dict[str, int] = {}
    by_coverage: dict[str, int] = {}
    for c in chips:
        by_vendor[c["vendor"]] = by_vendor.get(c["vendor"], 0) + 1
        by_family[c["family"]] = by_family.get(c["family"], 0) + 1
        by_coverage[c["coverage"]] = by_coverage.get(c["coverage"], 0) + 1

    return {
        "schema": "labwired.chip-catalog/v1",
        "summary": {
            "chips": len(chips),
            "public": sum(1 for c in chips if c["provenance"] == "public"),
            "private": private_count,
            "excluded": sum(1 for c in chips if c["status"] == "excluded"),
            "supported": sum(1 for c in chips if c["status"] == "supported"),
            "byVendor": dict(sorted(by_vendor.items())),
            "byFamily": dict(sorted(by_family.items())),
            "byCoverage": dict(sorted(by_coverage.items())),
        },
        "chips": chips,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--core-root",
        default=str(Path(__file__).resolve().parents[1]),
        help="labwired-core checkout root (default: repo containing this script)",
    )
    ap.add_argument(
        "--private-chips",
        default=os.environ.get("LABWIRED_IP_CHIPS", ""),
        help="dir of private chip descriptors (labwired-ip crates/labwired-ip-chips/chips)",
    )
    ap.add_argument(
        "--parity-tracker",
        default=os.environ.get("LABWIRED_IP_TRACKER", ""),
        help="path to labwired-ip parity-data/tracker.yaml",
    )
    ap.add_argument(
        "--out",
        default="",
        help="output JSON (default: <core-root>/docs/coverage/chips.json)",
    )
    ap.add_argument(
        "--require-private",
        action="store_true",
        help="fail when the private catalogue is absent instead of writing public-only",
    )
    ap.add_argument(
        "--publish-private",
        action="store_true",
        help=(
            "include private model names in the output. Off by default: this repo "
            "and docs.labwired.com are public, and the private catalogue is marked "
            "NOT FOR PUBLICATION, so naming the private parts is an explicit choice."
        ),
    )
    args = ap.parse_args()

    core_root = Path(args.core_root).resolve()
    out = Path(args.out) if args.out else core_root / "docs/coverage/chips.json"
    private_chips = Path(args.private_chips).resolve() if args.private_chips else None
    tracker = Path(args.parity_tracker).resolve() if args.parity_tracker else None

    have_private = bool(private_chips and private_chips.is_dir())
    if not have_private and args.require_private:
        sys.stderr.write(
            f"error: --require-private set but no private chip dir at {private_chips}\n"
        )
        return 2

    # The committed snapshot is the PUBLIC catalogue unless someone explicitly
    # asks to publish the private names. This is the default because the repo
    # and the docs site are public and the private catalogue is internal.
    if not args.publish_private:
        if have_private:
            sys.stderr.write(
                "notice: private catalogue found but --publish-private is off; "
                "writing the public catalogue only\n"
            )
        data = build(core_root, None, None)
    else:
        data = build(core_root, private_chips, tracker)

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    s = data["summary"]
    print(
        f"wrote {out} — {s['chips']} chips "
        f"({s['public']} public, {s['private']} private)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
