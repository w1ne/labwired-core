# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""chip_yaml.load_chip expands `include:` the way the Rust loader does."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import chip_yaml  # noqa: E402

REPO = Path(__file__).resolve().parents[1]


class LoadChipTest(unittest.TestCase):
    def test_variant_overlays_its_base(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "base.yaml").write_text(
                "name: base\narch: arm\nram: { base: 0x20000000 }\n"
                "pins:\n  P0: { gpio: sio, bit: 0 }\n"
                "peripherals:\n  - { id: a, type: stub }\n  - { id: b, type: x }\n"
            )
            (d / "variant.yaml").write_text(
                "include: base.yaml\nname: variant\n"
                "pins:\n  P1: { gpio: sio, bit: 33 }\n"
                "peripherals:\n  - { id: b, type: y }\n  - { id: c, type: z }\n"
            )
            chip = chip_yaml.load_chip(d / "variant.yaml")
        self.assertEqual(chip["name"], "variant")
        self.assertEqual(chip["arch"], "arm")
        self.assertNotIn("include", chip)
        self.assertEqual(sorted(chip["pins"]), ["P0", "P1"])
        self.assertEqual(
            [(p["id"], p["type"]) for p in chip["peripherals"]],
            [("a", "stub"), ("b", "y"), ("c", "z")],
        )

    def test_cycle_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "a.yaml").write_text("include: b.yaml\nname: a\n")
            (d / "b.yaml").write_text("include: a.yaml\nname: b\n")
            with self.assertRaises(ValueError):
                chip_yaml.load_chip(d / "a.yaml")

    def test_rp2354b_is_the_rp2350_die_with_qfn80_overlays(self) -> None:
        chips = REPO / "configs/chips"
        base = chip_yaml.load_chip(chips / "rp2350.yaml")
        chip = chip_yaml.load_chip(chips / "rp2354b.yaml")
        self.assertEqual(chip["arch"], base["arch"])
        self.assertEqual(chip["ram"], base["ram"])
        self.assertEqual(
            [p["id"] for p in chip["peripherals"]],
            [p["id"] for p in base["peripherals"]],
        )
        self.assertEqual(len(chip["pins"]), 48)


if __name__ == "__main__":
    unittest.main()
