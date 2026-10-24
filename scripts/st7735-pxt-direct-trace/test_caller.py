# SPDX-License-Identifier: MIT
import unittest
from unittest.mock import patch
import copy
import caller_run as caller


class CallerAdmissionControls(unittest.TestCase):
    def synthetic_capture(self, enabled):
        cases = []
        for index, name in enumerate(caller.CASE_NAMES):
            if index == 10:
                cases.append({"name": name, "inUpdateAfter": True, "copiedBytes": [],
                              "directFrames": 0, "fallbackCalls": 0})
                continue
            rejected = 7 <= index <= 9
            frames = (2 if index in [2, 3] else 1) if enabled and index < 4 else 0
            cases.append({
                "name": name, "rejected": rejected, "directFrames": frames,
                "fallbackCalls": 0 if rejected or frames else (2 if index in [2, 3] else 1),
                "mainPaddingSelected": enabled and index in [1, 3],
                "statusPaddingSelected": enabled and index == 3,
                "inUpdateAfter": rejected,
                "rejectionReason": ("screenBuf copy extent rejected" if index == 9
                                    else "dimension/bpp panic") if rejected else "",
                "frames": [{"csReleased": True, "ramwr": [0, 0, 0], "transfers": [3]}
                           for _ in range(frames)],
            })
        return {"rgb444Compiled": enabled, "cases": cases}

    def test_named_build_and_branch_verdicts_admitted(self):
        for enabled in [False, True]:
            caller.validate_capture(self.synthetic_capture(enabled), enabled)

    def test_missing_case_or_wrong_macro_rejected(self):
        capture = self.synthetic_capture(True)
        with self.assertRaisesRegex(RuntimeError, "macro identity"):
            caller.validate_capture(capture, False)
        capture["cases"].pop()
        with self.assertRaisesRegex(RuntimeError, "missing caller"):
            caller.validate_capture(capture, True)

    def test_branch_padding_rejection_and_frame_mutants_rejected(self):
        original = self.synthetic_capture(True)
        mutants = [
            (0, "directFrames", 0),
            (1, "mainPaddingSelected", False),
            (3, "statusPaddingSelected", False),
            (7, "rejectionReason", "unrelated failure"),
            (0, "inUpdateAfter", True),
            (0, "frames", [{"csReleased": False, "ramwr": [0], "transfers": [1]}]),
            (0, "frames", [{"csReleased": True, "ramwr": [0], "transfers": [2]}]),
            (0, "frames", [{"csReleased": True, "ramwr": [], "transfers": []}]),
            (0, "frames", [{"csReleased": True, "ramwr": [256, 0, 0], "transfers": [3]}]),
            (0, "frames", [{"csReleased": True, "ramwr": [0, 0, 0], "transfers": [0, 3]}]),
            (10, "copiedBytes", [1]),
        ]
        for index, key, value in mutants:
            with self.subTest(index=index, key=key, value=value):
                capture = copy.deepcopy(original)
                capture["cases"][index][key] = value
                with self.assertRaises(RuntimeError):
                    caller.validate_capture(capture, True)

    def test_original_header_pin(self):
        self.assertEqual(caller.HEADER, "libs/base/pxtbase.h")
        self.assertEqual(
            caller.HEADER_SHA,
            "87c459c4d8fefa5bd851f862be8e83659185825c41448155caed34a22fe0bd17",
        )

    def test_exact_fragment_contract(self):
        self.assertEqual(caller.FRAGMENTS, {
            "caller.inc": "9443b0d8e64b57daf36068a0ca8a50a2c58094452573c8dabe7522a3e7a40236",
            "image-header.inc": "23056e13fc8341d15f3754024000f35f8928b2dd5ea7aa916f1fd47072cdac4b",
            "image-accessors.inc": "8198b2664d5627e90b54d477f11751c7788b06e1d9abfe0d345bf02da5f26436",
            "method.inc": caller.direct.FRAGMENTS["sendIndexedImage444"],
        })

    def test_changed_fragment_cannot_be_admitted(self):
        with patch.object(caller.direct, "fragment", return_value=b"changed"):
            with self.assertRaisesRegex(RuntimeError, "pinned caller/accessor fragment mismatch"):
                caller.extract(b"synthetic", b"synthetic")


if __name__ == "__main__":
    unittest.main()
