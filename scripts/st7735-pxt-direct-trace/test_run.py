# SPDX-License-Identifier: MIT
"""Runner admission controls only: no network, compiler or renderer execution."""
import hashlib
import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("pxt_direct_runner", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class AdmissionControls(unittest.TestCase):
    def test_fragment_preserves_raw_bytes(self):
        self.assertEqual(
            runner.fragment(b"prefix START\r\n\x00body\r\nEND suffix", b"START", b"END"),
            b"START\r\n\x00body\r\n",
        )

    def test_duplicate_start_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "ambiguous"):
            runner.fragment(b"START one END START two END", b"START", b"END")

    def test_missing_start_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "ambiguous"):
            runner.fragment(b"body END", b"START", b"END")

    def test_missing_end_is_rejected(self):
        with self.assertRaises(ValueError):
            runner.fragment(b"START body", b"START", b"END")

    def test_end_before_start_cannot_select_wrong_extent(self):
        self.assertEqual(
            runner.fragment(b"END prefix START body END tail", b"START", b"END"),
            b"START body ",
        )

    def downloaded(self, data, wanted):
        with patch.object(runner, "INPUTS", {"synthetic": wanted}), patch.object(
            runner.urllib.request, "urlopen", return_value=io.BytesIO(data)
        ) as request:
            result = runner.download_inputs()
            request.assert_called_once_with(
                f"https://raw.githubusercontent.com/microsoft/pxt-common-packages/{runner.PIN}/synthetic",
                timeout=30,
            )
            return result

    def test_download_preserves_sha_bound_raw_bytes(self):
        data = b"synthetic fixture\r\n\x00"
        self.assertEqual(self.downloaded(data, hashlib.sha256(data).hexdigest()), {"synthetic": data})

    def test_corrupt_download_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "pinned input mismatch"):
            self.downloaded(b"corrupt", hashlib.sha256(b"expected").hexdigest())

    def test_oversized_download_is_rejected_even_with_matching_hash(self):
        data = b"x" * 65537
        with self.assertRaisesRegex(RuntimeError, "pinned input mismatch"):
            self.downloaded(data, hashlib.sha256(data).hexdigest())

    def test_network_failure_cannot_produce_admitted_input(self):
        with patch.object(runner.urllib.request, "urlopen", side_effect=OSError("synthetic failure")):
            with self.assertRaisesRegex(OSError, "synthetic failure"):
                runner.download_inputs()

    def test_declared_production_pins_are_exact(self):
        self.assertEqual(runner.PIN, "31abf23d118f35010fb75122e60a1eb2b6dffe9f")
        self.assertEqual(runner.INPUTS, {
            "libs/screen---st7735/screen.cpp":
                "dea9ea175d65d885275eb0715d56353674feae88d64d29a5eb2809f899a0d958",
            "LICENSE": "dea9265341829002e2c23a7372393eb2ed6e26085fb623f38a4ba0af833f30a6",
        })
        self.assertEqual(runner.FRAGMENTS, {
            "sendIndexedImage444": "95429c6b140c357228bd9a2f56a65cafca75d83b7b2f56fa338bba25f3bdce05",
            "boardIdPredicate": "89f00c6fbca7dc68e7fdee9a2a459f61a5beb0e8b5337df7adaefa70523c8a71",
        })


if __name__ == "__main__":
    unittest.main()
