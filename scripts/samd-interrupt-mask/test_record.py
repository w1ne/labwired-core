#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Synthetic receipt controls, not guest execution or independent evidence."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import unittest
from unittest.mock import patch

HEAD = '4' * 40
FIXTURE = 'e15482fd6b537ffd208919c31d568c0bea14d47a'
POSITIVE, MUTANT = b'synthetic-positive', b'synthetic-mutant'
POSITIVE_HASH = hashlib.sha256(POSITIVE).hexdigest()
MUTANT_HASH = hashlib.sha256(MUTANT).hexdigest()
VALID = (
    f'SAMD_INTERRUPT_MASK case=positive elf_sha256={POSITIVE_HASH} status=0x600d steps=315 bss_clear=true entry=true\n'
    f'SAMD_INTERRUPT_MASK case=restore-enabled elf_sha256={MUTANT_HASH} status=0x3 steps=126 bss_clear=true entry=true\n'
)


class ReceiptControls(unittest.TestCase):
    def exercise(self, case):
        log = VALID
        if case == 'status':
            log = log.replace('status=0x3', 'status=0x4')
        elif case == 'hash':
            log = log.replace(POSITIVE_HASH, '0' * 64)
        elif case == 'missing':
            log = log.splitlines()[0] + '\n'

        def read_bytes(path):
            name = str(path)
            if name.startswith('mask-guest-'):
                return log.encode()
            if name.endswith('/positive/control.elf'):
                return POSITIVE
            if name.endswith('/restore-enabled/control.elf'):
                return MUTANT
            raise AssertionError(name)

        def git_output(args, **kwargs):
            if args[1:3] == ['-C', 'owned-mask-source']:
                return ('0' * 40 if case == 'fixture' else FIXTURE) + '\n'
            if args[1:] == ['cat-file', '-p', 'HEAD']:
                second = '5' * 40 if case == 'parent' else HEAD
                parents = f'parent {"1" * 40}\nparent {second}\n'
                if case == 'single-parent':
                    parents = f'parent {HEAD}\n'
                if case == 'malformed-parent':
                    parents = 'parent not-an-object\n'
                # A message line resembling another parent must be ignored.
                return f'tree {"3" * 40}\n{parents}author synthetic\n\nparent {"9" * 40}\n'
            if args[-1] == 'HEAD^{tree}':
                return '3' * 40 + '\n'
            if args[1:] == ['rev-parse', 'HEAD']:
                return '2' * 40 + '\n'
            # git show --format=%P would return empty on the original shallow
            # checkout. Forbid it: the regression must inspect raw headers.
            raise AssertionError(f'Unexpected Git command: {args}')

        written = []
        env = {'QUALIFICATION_HEAD': HEAD, 'GITHUB_RUN_ID': '1',
               'GITHUB_RUN_ATTEMPT': '1', 'RUNNER_TEMP': 'synthetic'}
        with patch.dict(os.environ, env), patch('subprocess.check_output', git_output), \
                patch.object(Path, 'read_bytes', read_bytes), \
                patch.object(Path, 'write_text', lambda p, s: written.append(json.loads(s))):
            if case == 'valid':
                runpy.run_path(str(Path(__file__).with_name('record.py')))
                self.assertEqual(written[0]['parents'], ['1' * 40, HEAD])
            else:
                with self.assertRaises(AssertionError):
                    runpy.run_path(str(Path(__file__).with_name('record.py')))
                self.assertEqual(written, [])

    def test_shallow_boundary_uses_raw_header(self):
        self.exercise('valid')

    def test_invalid_bindings_refused(self):
        for case in ['status', 'hash', 'missing', 'fixture', 'parent',
                     'single-parent', 'malformed-parent']:
            with self.subTest(case=case):
                self.exercise(case)


if __name__ == '__main__':
    unittest.main()
