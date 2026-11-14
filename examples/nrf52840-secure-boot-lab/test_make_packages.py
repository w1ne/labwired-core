"""Regressions for the P-256 SubjectPublicKeyInfo public-key extraction."""

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().with_name("make_packages.py")
SPEC = importlib.util.spec_from_file_location("make_packages", SCRIPT)
PACKAGES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGES)
PREFIX = bytes.fromhex("3059301306072a8648ce3d020106082a8648ce3d03010703420004")


class PublicKeyExtractionTests(unittest.TestCase):
    def extract(self, der):
        result = subprocess.CompletedProcess([], 0, stdout=der, stderr=b"")
        with patch.object(PACKAGES.subprocess, "run", return_value=result):
            return PACKAGES.pubkey_xy_from_private(Path("unused.pem"))

    def test_marker_byte_inside_either_coordinate_is_data(self):
        for offset in range(64):
            with self.subTest(offset=offset):
                coordinates = bytearray([0x42] * 64)
                coordinates[offset] = 0x04
                self.assertEqual(self.extract(PREFIX + coordinates), coordinates)

    def test_truncated_and_extended_points_are_rejected(self):
        for length in (0, 63, 65):
            with self.subTest(length=length), self.assertRaises(RuntimeError):
                self.extract(PREFIX + bytes([0x42] * length))

    def test_wrong_curve_identifier_is_rejected(self):
        prefix = bytearray(PREFIX)
        prefix[22] ^= 1
        with self.assertRaises(RuntimeError):
            self.extract(prefix + bytes([0x42] * 64))


if __name__ == "__main__":
    unittest.main()
