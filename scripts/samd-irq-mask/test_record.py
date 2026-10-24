#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Mocked production-receipt controls; not guest or independent evidence."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import unittest
from unittest.mock import patch

HEAD = '4' * 40
FIXTURE = '2a991760339fe89f5272f923baf89dc18fd434d0'
ELFS = {'positive': b'synthetic-positive', 'restore-enabled': b'synthetic-mutant'}
VALID = ''.join(
    f'SAMD_IRQ_MASK case={case} elf_sha256={hashlib.sha256(ELFS[case]).hexdigest()} '
    f'status={status} steps=100 bss_clear=true entry=true vectors=true irq_count=1 '
    f'stage={stage} mask=0 exception=16 irq_entry=true irq_return=true\n'
    for case, status, stage in [('positive', '0x600d', 2), ('restore-enabled', '0x6', 1)]
)


class ReceiptControls(unittest.TestCase):
    def test_source_and_witness_binding(self):
        for case in ['valid', 'status', 'hash', 'missing', 'stage', 'irq-entry',
                     'fixture', 'parent', 'malformed-parent']:
            with self.subTest(case=case):
                log = VALID
                if case == 'status':
                    log = log.replace('status=0x6 ', 'status=0x7 ')
                elif case == 'hash':
                    log = log.replace(hashlib.sha256(ELFS['positive']).hexdigest(), '0' * 64)
                elif case == 'missing':
                    log = log.splitlines()[0] + '\n'
                elif case == 'stage':
                    log = log.replace('stage=1 ', 'stage=2 ')
                elif case == 'irq-entry':
                    log = log.replace('irq_entry=true', 'irq_entry=false')

                def read_bytes(path):
                    name = str(path)
                    if name.startswith('irq-guest-'):
                        return log.encode()
                    for elf_case, data in ELFS.items():
                        if name.endswith(f'/{elf_case}/control.elf'):
                            return data
                    raise AssertionError(name)

                def git_output(args, **kwargs):
                    if args[1:3] == ['-C', 'owned-irq-source']:
                        return ('0' * 40 if case == 'fixture' else FIXTURE) + '\n'
                    if args[1:] == ['cat-file', '-p', 'HEAD']:
                        parent = '5' * 40 if case == 'parent' else HEAD
                        if case == 'malformed-parent':
                            parent = 'not-a-sha'
                        return f'tree {"3" * 40}\nparent {"1" * 40}\nparent {parent}\n\nparent {"9" * 40}\n'
                    if args[1:] == ['rev-parse', 'HEAD^{tree}']:
                        return '3' * 40 + '\n'
                    if args[1:] == ['rev-parse', 'HEAD']:
                        return '2' * 40 + '\n'
                    raise AssertionError(f'Unexpected Git command: {args}')

                written = []
                env = {'QUALIFICATION_HEAD': HEAD, 'GITHUB_RUN_ID': '1',
                       'GITHUB_RUN_ATTEMPT': '1', 'RUNNER_TEMP': 'synthetic'}
                with patch.dict(os.environ, env), patch('subprocess.check_output', git_output), \
                        patch.object(Path, 'read_bytes', read_bytes), \
                        patch.object(Path, 'write_text', lambda p, s: written.append(json.loads(s))):
                    script = str(Path(__file__).with_name('record.py'))
                    if case == 'valid':
                        runpy.run_path(script)
                        self.assertEqual(written[0]['parents'], ['1' * 40, HEAD])
                    else:
                        with self.assertRaises(AssertionError):
                            runpy.run_path(script)
                        self.assertEqual(written, [])


if __name__ == '__main__':
    unittest.main()
