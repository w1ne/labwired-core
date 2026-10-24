#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Manufactured result-reader controls only; never actual guest testimony."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import mock_open, patch

spec = importlib.util.spec_from_file_location('timer_record', Path(__file__).with_name('record.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReaderControls(unittest.TestCase):
    def fixture(self):
        elf, mapping, source = b'manufactured ELF', b'manufactured map', 'a' * 40
        before = {'schema': 1, 'samd_commit': source, 'core_commit': module.CORE_PIN,
                  'files': {name: module.record(b'manufactured source') for name in module.FILES}}
        build = {'schema': 1, 'source': copy.deepcopy(before), 'elf': module.record(elf),
                 'map': module.record(mapping), 'guest_executed': False, 'runtime_admitted': False,
                 'units': {'timer': sorted('core/' + name for name in module.CORE_FILES if name != 'LICENSE'),
                           'profile': ['samd/paired-irq/profile.cpp'], 'probe': ['samd/timer-caller/probe.cpp']}}
        row = (f'SAMD_REAL_TIMER elf_sha256={module.record(elf)["sha256"]} source={source} steps=123 '
               'bss_clear=true data_copy=true vectors=true milestones=6 irq_entries=2 irq_returns=2 '
               'handler_mask=0 handler_exception=16 paired_closed=true fault=0 status=0x600d '
               'final_mask=1 final_exception=0\n').encode()
        return {'default': row, 'scheduler': row}, elf, mapping, build, before, source

    def test_manufactured_complete_profiles_accept(self):
        self.assertEqual(set(module.validate(*self.fixture())), {'default', 'scheduler'})

    def test_missing_duplicate_and_corrupt_witness_refuse(self):
        for mode in ('missing', 'duplicate', 'mask', 'progress', 'timeout', 'zero', 'source', 'elf', 'return'):
            args = list(self.fixture()); logs = args[0]
            if mode == 'missing': del logs['scheduler']
            elif mode == 'duplicate': logs['default'] *= 2
            else:
                changes = {'mask': (b'handler_mask=0', b'handler_mask=1'),
                           'progress': (b'milestones=6', b'milestones=5'),
                           'timeout': (b'steps=123', b'steps=250001'), 'zero': (b'steps=123', b'steps=0'),
                           'source': (b'source=' + b'a'*40, b'source=' + b'b'*40),
                           'elf': (module.record(args[1])['sha256'].encode(), b'f'*64),
                           'return': (b'irq_returns=2', b'irq_returns=1')}
                logs['default'] = logs['default'].replace(*changes[mode])
            with self.subTest(mode=mode), self.assertRaises(ValueError): module.validate(*args)

    def test_source_inventory_drift_and_admission_refuse(self):
        for mode in ('inventory', 'drift', 'admission', 'elf', 'map', 'headers', 'unit', 'bytes'):
            args = list(self.fixture()); build, before = args[3:5]
            if mode == 'inventory':
                before['files']['private/unknown'] = module.record(b'injected'); build['source'] = copy.deepcopy(before)
            elif mode == 'drift': build['source']['samd_commit'] = 'b'*40
            elif mode == 'admission': build['runtime_admitted'] = True
            elif mode == 'elf': args[1] = b'changed ELF'
            elif mode == 'map': args[2] = b'changed map'
            elif mode == 'headers': build['units']['timer'].remove('core/inc/types/Event.h')
            elif mode == 'unit': build['units']['probe'] = ['samd/paired-irq/profile.cpp']
            else:
                name = next(iter(before['files'])); before['files'][name]['bytes'] = True
                build['source'] = copy.deepcopy(before)
            with self.subTest(mode=mode), self.assertRaises(ValueError): module.validate(*args)

    def run_entrypoint(self, mode=None):
        logs, elf, mapping, build, before, source = self.fixture()
        head, base = 'c' * 40, 'd' * 40
        env = {'LABWIRED_TIMER_QUALIFIED_SOURCE': source,
               'LABWIRED_TIMER_FIXTURE_DIR': '/manufactured/fixture',
               'LABWIRED_TIMER_SOURCE_DIR': '/manufactured/samd',
               'LABWIRED_TIMER_CORE_DIR': '/manufactured/core',
               'QUALIFICATION_HEAD': head, 'GITHUB_REPOSITORY': 'w1ne/labwired-core',
               'GITHUB_EVENT_NAME': 'pull_request', 'GITHUB_RUN_ID': '123',
               'GITHUB_RUN_ATTEMPT': '1'}
        if mode == 'repository': env['GITHUB_REPOSITORY'] = 'other/repository'
        elif mode == 'event': env['GITHUB_EVENT_NAME'] = 'workflow_dispatch'
        elif mode == 'run': env['GITHUB_RUN_ID'] = '0'
        elif mode == 'attempt': env['GITHUB_RUN_ATTEMPT'] = '0'
        def read_bytes(path):
            if path.name == 'build-provenance.json': return json.dumps(build).encode()
            if path.name == 'source-before.json': return json.dumps(before).encode()
            if path.name == 'control.elf': return elf
            if path.name == 'control.map': return mapping
            if path.name.startswith('timer-guest-'): return logs[path.stem.removeprefix('timer-guest-')]
            if mode == 'live_bytes' and path.name == 'probe.cpp': return b'changed live source'
            return b'manufactured source'
        def git(*args):
            if args[:2] == ('-C', '/manufactured/samd'):
                return 'b' * 40 if mode == 'live_pin' else source
            if args[:2] == ('-C', '/manufactured/core'): return module.CORE_PIN
            if args[0] == 'show':
                if mode == 'one_parent': return base
                if mode == 'wrong_head': return base + ' ' + 'e' * 40
                return base + ' ' + head
            return 'f' * 40 if args[-1] == 'HEAD' else '9' * 40
        output = mock_open()
        if mode == 'existing_receipt': output.side_effect = FileExistsError('existing receipt')
        with patch.dict(module.os.environ, env, clear=True), \
                patch.object(Path, 'read_bytes', read_bytes), patch.object(module, 'git', git), \
                patch.object(module.subprocess, 'check_output', return_value=b'manufactured source'), \
                patch.object(Path, 'open', output):
            if mode:
                with self.assertRaises(FileExistsError if mode == 'existing_receipt' else ValueError):
                    module.main()
                output.return_value.write.assert_not_called()
                if mode != 'existing_receipt': output.assert_not_called()
                return
            module.main()
        output.assert_called_once_with('x')
        receipt = json.loads(output.return_value.write.call_args.args[0])
        self.assertEqual(receipt['head'], head)
        self.assertEqual(receipt['parents'], [base, head])
        self.assertEqual(receipt['run'], 123)
        self.assertEqual(receipt['attempt'], 1)
        self.assertEqual(set(receipt['profiles']), {'default', 'scheduler'})

    def test_manufactured_entrypoint_exclusive_receipt(self):
        self.run_entrypoint()

    def test_entrypoint_refusals_write_nothing(self):
        for mode in ('repository', 'event', 'run', 'attempt', 'live_bytes', 'live_pin',
                     'one_parent', 'wrong_head', 'existing_receipt'):
            with self.subTest(mode=mode): self.run_entrypoint(mode)


if __name__ == '__main__':
    unittest.main()
