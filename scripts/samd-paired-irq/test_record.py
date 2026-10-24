#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Manufactured reader controls; not actual or independent guest evidence."""
import copy
import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch
import record
from record import EXPECTED, SOURCE_FILES, sha, validate

SOURCE = '1' * 40
ELFS = {mode: f'synthetic-elf-{mode}'.encode() for mode in range(5)}
METADATA = {'schema': 1, 'source_commit': SOURCE, 'compiler_sha256': '2' * 64,
            'source_files': {name: {'sha256': '3' * 64, 'bytes': 1} for name in SOURCE_FILES}}
VALID = ''.join(
    f'SAMD_PAIRED_IRQ mode={mode} elf_sha256={sha(ELFS[mode])} status={status} '
    f'fault={fault} steps=100 bss_clear=true data_copy=true entry=true vectors=true '
    f'irq_entries={entries} stage={stage} handler_mask=0 handler_exception={exception} '
    f'final_mask=1 final_exception=0 refusal_steps={refusal}\n'
    for mode, status, fault, entries, stage, exception, refusal in EXPECTED
).encode()


class ReaderControls(unittest.TestCase):
    def test_snapshot_source_and_driver_binding(self):
        source = Path('synthetic-source')
        for case in ('valid', 'pin', 'malformed-pin', 'dirty', 'working-bytes', 'compiler'):
            with self.subTest(case=case):
                expected = 'not-a-sha' if case == 'malformed-pin' else SOURCE

                def git_output(*args):
                    if args == ('-C', str(source), 'rev-parse', 'HEAD'):
                        return '5' * 40 if case == 'pin' else SOURCE
                    if args == ('-C', str(source), 'status', '--porcelain', '--untracked-files=all'):
                        return ' M paired-irq/profile.cpp' if case == 'dirty' else ''
                    raise AssertionError(args)

                def original_bytes(args):
                    self.assertEqual(args[:5], ['git', '-C', str(source), 'show', args[4]])
                    self.assertIn(args[4], ['HEAD:' + name for name in SOURCE_FILES])
                    return b'owned-source'

                def read_bytes(path):
                    if str(path) == 'synthetic-driver': return b'synthetic-compiler-driver'
                    self.assertIn(str(path), [str(source / name) for name in SOURCE_FILES])
                    if case == 'working-bytes' and path == source / SOURCE_FILES[0]: return b'altered'
                    return b'owned-source'

                with patch.object(record, 'git', git_output), \
                        patch('subprocess.check_output', original_bytes), \
                        patch.object(Path, 'read_bytes', read_bytes), \
                        patch('shutil.which', return_value=None if case == 'compiler' else 'synthetic-driver'):
                    if case == 'valid':
                        result = record.snapshot(source, expected)
                        self.assertEqual(result['source_commit'], SOURCE)
                        self.assertEqual(set(result['source_files']), set(SOURCE_FILES))
                        self.assertEqual(result['compiler_sha256'], sha(b'synthetic-compiler-driver'))
                    else:
                        with self.assertRaises(ValueError): record.snapshot(source, expected)

    def test_recording_entry_point_bindings(self):
        head = '4' * 40
        for case in ('valid', 'parent', 'malformed-parent', 'extra-parent', 'head',
                     'after', 'snapshot', 'source', 'run', 'attempt'):
            with self.subTest(case=case):
                env = {'LABWIRED_PAIRED_QUALIFIED_SOURCE': SOURCE,
                       'LABWIRED_PAIRED_SOURCE_DIR': 'synthetic-source',
                       'LABWIRED_PAIRED_FIXTURE_DIR': 'synthetic-fixtures',
                       'QUALIFICATION_HEAD': head, 'GITHUB_RUN_ID': '1', 'GITHUB_RUN_ATTEMPT': '1'}
                if case == 'head': env['QUALIFICATION_HEAD'] = '5' * 40
                if case == 'source': env['LABWIRED_PAIRED_QUALIFIED_SOURCE'] = '5' * 40
                if case == 'run': env['GITHUB_RUN_ID'] = '0'
                if case == 'attempt': env['GITHUB_RUN_ATTEMPT'] = '0'
                parent = '5' * 40 if case == 'parent' else head
                if case == 'malformed-parent': parent = 'invalid'
                headers = f'tree {"6" * 40}\nparent {"7" * 40}\nparent {parent}\n'
                if case == 'extra-parent': headers += f'parent {"8" * 40}\n'
                # Parent-shaped commit-message text must not become a third parent.
                headers += f'\nparent {"9" * 40}\n'

                def git_output(*args):
                    if args == ('cat-file', '-p', 'HEAD'): return headers
                    if args == ('rev-parse', 'HEAD'): return 'a' * 40
                    if args == ('rev-parse', 'HEAD^{tree}'): return '6' * 40
                    raise AssertionError(args)

                def read_bytes(path):
                    name = str(path)
                    if name.endswith('source-before.json'): return json.dumps(METADATA).encode()
                    if name.endswith('source-after.json'):
                        return json.dumps(METADATA).encode() + (b' ' if case == 'after' else b'')
                    if name.startswith('paired-guest-'): return VALID
                    for mode, data in ELFS.items():
                        if name.endswith(f'/{mode}/control.elf'): return data
                    raise AssertionError(name)

                fresh = copy.deepcopy(METADATA)
                if case == 'snapshot': fresh['compiler_sha256'] = 'b' * 64
                written = []
                with patch.dict(os.environ, env), patch.object(record, 'git', git_output), \
                        patch.object(record, 'snapshot', return_value=fresh), \
                        patch.object(Path, 'read_bytes', read_bytes), \
                        patch.object(Path, 'write_text', lambda p, s: written.append(json.loads(s))), \
                        patch('sys.argv', ['record.py']):
                    if case == 'valid':
                        record.main()
                        self.assertEqual(len(written), 1)
                        self.assertEqual(written[0]['parents'], ['7' * 40, head])
                        self.assertEqual(written[0]['profiles']['default']['results'][0]['status'], '0x600d')
                    else:
                        with self.assertRaises(ValueError): record.main()
                        self.assertEqual(written, [], 'refusal must not publish a receipt')

    def test_manufactured_positive(self):
        result = validate({'default': VALID, 'scheduler': VALID}, ELFS, METADATA, SOURCE)
        self.assertEqual(len(result['scheduler']['results']), 5)
        self.assertEqual(result['default']['results'][4]['fault'], 6)

    def test_all_profiles_reject_each_missing_or_corrupted_result(self):
        for profile in ('default', 'scheduler'):
            for mode in range(5):
                for mutation in ('missing', 'duplicate', 'hash', 'status', 'fault', 'steps',
                                 'bss', 'data', 'entry', 'vectors', 'irq', 'stage', 'mask',
                                 'exception', 'final-mask', 'final-exception', 'refusal'):
                    with self.subTest(profile=profile, mode=mode, mutation=mutation):
                        lines = VALID.decode().splitlines()
                        if mutation == 'missing':
                            del lines[mode]
                        elif mutation == 'duplicate':
                            lines.insert(mode, lines[mode])
                        else:
                            old, new = {
                                'hash': (f'elf_sha256={sha(ELFS[mode])}', f'elf_sha256={"0" * 64}'),
                                'status': (f'status={EXPECTED[mode][1]}', 'status=0xbad'),
                                'fault': (f'fault={EXPECTED[mode][2]}', 'fault=99'),
                                'steps': ('steps=100 ', 'steps=100001 '),
                                'bss': ('bss_clear=true', 'bss_clear=false'),
                                'data': ('data_copy=true', 'data_copy=false'),
                                'entry': ('entry=true', 'entry=false'),
                                'vectors': ('vectors=true', 'vectors=false'),
                                'irq': (f'irq_entries={EXPECTED[mode][3]}', 'irq_entries=99'),
                                'stage': (f'stage={EXPECTED[mode][4]}', 'stage=99'),
                                'mask': ('handler_mask=0', 'handler_mask=1'),
                                'exception': (f'handler_exception={EXPECTED[mode][5]}', 'handler_exception=99'),
                                'final-mask': ('final_mask=1', 'final_mask=0'),
                                'final-exception': ('final_exception=0', 'final_exception=16'),
                                'refusal': (f'refusal_steps={EXPECTED[mode][6]}', 'refusal_steps=63'),
                            }[mutation]
                            self.assertEqual(lines[mode].count(old), 1)
                            lines[mode] = lines[mode].replace(old, new)
                        logs = {'default': VALID, 'scheduler': VALID}
                        logs[profile] = ('\n'.join(lines) + '\n').encode()
                        with self.assertRaises(ValueError):
                            validate(logs, ELFS, METADATA, SOURCE)

    def test_source_fixture_and_profile_refusals(self):
        for case in ('schema', 'profile', 'source', 'malformed-source', 'file', 'compiler', 'empty-elf',
                     'missing-elf', 'extra-elf', 'reordered', 'extra-diagnostic'):
            with self.subTest(case=case):
                logs = {'default': VALID, 'scheduler': VALID}
                elfs = ELFS.copy()
                metadata = copy.deepcopy(METADATA)
                source = SOURCE
                if case == 'schema': metadata['schema'] = 99
                elif case == 'profile': del logs['scheduler']
                elif case == 'source': metadata['source_commit'] = '4' * 40
                elif case == 'malformed-source': source = 'not-a-sha'
                elif case == 'file': del metadata['source_files'][SOURCE_FILES[0]]
                elif case == 'compiler': metadata['compiler_sha256'] = 'bad'
                elif case == 'empty-elf': elfs[0] = b''
                elif case == 'missing-elf': del elfs[0]
                elif case == 'extra-elf': elfs[5] = b'unknown'
                elif case == 'reordered': logs['default'] = b'\n'.join(reversed(VALID.splitlines())) + b'\n'
                elif case == 'extra-diagnostic': logs['default'] += b'SAMD_PAIRED_IRQ malformed\n'
                with self.assertRaises(ValueError):
                    validate(logs, elfs, metadata, source)


if __name__ == '__main__':
    unittest.main()
