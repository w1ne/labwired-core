#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Manufactured source selectors only; no compiler or guest execution."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('timer_mutants', Path(__file__).with_name('build.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class SelectorControls(unittest.TestCase):
    def test_exact_unique_owned_source_changes_once(self):
        for case, (_, selector, replacement, _) in module.CASES.items():
            with self.subTest(case=case):
                original = ('prefix\n' + selector + '\nsuffix').encode()
                self.assertEqual(module.mutate(original, case),
                                 ('prefix\n' + replacement + '\nsuffix').encode())

    def test_missing_or_ambiguous_site_refuses(self):
        for case, (_, selector, _, _) in module.CASES.items():
            for original in (b'no selector', (selector + '\n' + selector).encode()):
                with self.subTest(case=case), self.assertRaises(ValueError):
                    module.mutate(original, case)

if __name__ == '__main__':
    unittest.main()
