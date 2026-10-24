#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Manufactured negative producer controls only, not guest testimony."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import mock_open, patch

spec = importlib.util.spec_from_file_location('negative_record', Path(__file__).with_name('record.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class ReaderControls(unittest.TestCase):
    def test_recorded_compiler_component_drift_refuses(self):
        before = self.fixture()[2]
        after = {label: before['files'][label] for label in module.COMPILER_FILES}
        module.verify_compiler(after, before)
        for label in module.COMPILER_FILES:
            changed = copy.deepcopy(after); changed[label]['sha256'] = 'f'*64
            with self.subTest(label=label), self.assertRaises(ValueError): module.verify_compiler(changed, before)
        changed = dict(after); changed['unknown/compiler'] = module.identity(b'unknown')
        with self.assertRaises(ValueError): module.verify_compiler(changed, before)

    def test_manufactured_compiler_snapshot_binds_three_actual_paths(self):
        def command(args):
            return b'/manufactured/libgcc.a' if args[-1] == '-print-libgcc-file-name' else b'/manufactured/include'
        with patch.object(module.shutil, 'which', return_value='/manufactured/driver'), \
                patch.object(module.subprocess, 'check_output', command), \
                patch.object(Path, 'read_bytes', lambda path: str(path).encode()):
            result = module.compiler_snapshot()
        self.assertEqual(result['compiler/libgcc.a'], module.identity(b'/manufactured/libgcc.a'))
        self.assertEqual(result['compiler/include/stdbool.h'], module.identity(b'/manufactured/include/stdbool.h'))
        self.assertEqual(result['compiler/driver'], module.identity(b'/manufactured/driver'))
    def fixture(self):
        originals = {label: b'manufactured source' for label in module.FILES - module.COMPILER_FILES}
        originals['samd/paired-irq/profile.cpp'] = module.LEAVE.encode() + b'\n'
        originals['samd/timer-caller/probe.cpp'] = module.ENTRY.encode() + b'\n'
        before = {'schema': 1, 'samd_commit': module.SAMD, 'core_commit': module.CORE,
                  'files': {label: module.identity(originals.get(label, b'manufactured compiler')) for label in module.FILES}}
        images = {'elf': b'manufactured positive ELF', 'map': b'manufactured positive map'}
        build = {'schema': 1, 'source': copy.deepcopy(before), **{field: module.identity(data) for field, data in images.items()},
                 'guest_executed': False, 'runtime_admitted': False,
                 'units': {'timer': sorted('core/' + p for p in module.CORE_FILES if p != 'LICENSE'),
                           'profile': ['samd/paired-irq/profile.cpp'], 'probe': ['samd/timer-caller/probe.cpp']}}
        positive = {name: ('manufactured object ' + name).encode() for name in ('startup', 'timer', 'profile', 'probe')}
        mutants, rows = {}, []
        for case, (path, unit, selector, replacement, status, exception, entries, milestones) in module.CASES.items():
            source = originals['samd/' + path].replace(selector.encode(), replacement.encode())
            objects = dict(positive); objects[unit] = ('manufactured changed object ' + case).encode()
            elf, mapping = ('manufactured ELF ' + case).encode(), ('manufactured map ' + case).encode()
            manifest = {'schema': 1, 'case': case, 'samd_commit': module.SAMD, 'guest_executed': False, 'runtime_admitted': False,
                        'original_source': {'label': 'samd/' + path, **module.identity(originals['samd/' + path])},
                        'generated_source': module.identity(source), 'replaced_unit': unit,
                        'dependencies': ['generated/' + unit], 'expected_failure': {'status': status, 'fault': 0, 'exception': exception},
                        'objects': {name: module.identity(data) for name, data in objects.items()},
                        'elf': module.identity(elf), 'map': module.identity(mapping)}
            mutants[case] = {'manifest': manifest, 'source': source, 'objects': objects, 'elf': elf, 'map': mapping}
            rows.append(f'SAMD_TIMER_REFUSAL case={case} elf_sha256={module.identity(elf)["sha256"]} steps=123 terminal=64 '
                        f'status={status} fault=0 mask=1 exception={exception} irq_entries={entries} milestones={milestones} startup=true')
        log = ('\n'.join(rows) + '\n').encode()
        return [{'default': log, 'scheduler': log}, build, before, originals, positive, images, mutants]

    def test_complete_manufactured_six_cases_accept(self):
        result = module.validate(*self.fixture())
        self.assertEqual(set(result), {'default', 'scheduler'})
        self.assertTrue(all(len(p['cases']) == 3 for p in result.values()))

    def test_missing_corrupt_duplicate_wrong_or_stalled_observation_refuses(self):
        changes = [('terminal=64', 'terminal=63'), ('steps=123', 'steps=250001'), ('steps=123', 'steps=0'),
                   ('mask=1', 'mask=0'), ('fault=0', 'fault=6'), ('exception=16', 'exception=0'),
                   ('irq_entries=1', 'irq_entries=0'), ('milestones=1', 'milestones=0'),
                   ('status=0xb02', 'status=0x600d'), ('startup=true', 'startup=false')]
        for old, new in changes:
            args = self.fixture(); args[0]['default'] = args[0]['default'].replace(old.encode(), new.encode())
            with self.subTest(old=old, new=new), self.assertRaises(ValueError): module.validate(*args)
        for mode in ('missing_profile', 'duplicate_case', 'missing_case', 'unknown_case', 'wrong_elf'):
            args = self.fixture(); logs = args[0]
            if mode == 'missing_profile': del logs['scheduler']
            elif mode == 'duplicate_case': logs['default'] += logs['default'].splitlines(keepends=True)[0]
            elif mode == 'missing_case': logs['default'] = b''.join(logs['default'].splitlines(keepends=True)[:2])
            elif mode == 'unknown_case': logs['default'] = logs['default'].replace(b'unconditional_restore', b'unknown_case')
            else: logs['default'] = logs['default'].replace(module.identity(args[-1]['early_restore']['elf'])['sha256'].encode(), b'f'*64)
            with self.subTest(mode=mode), self.assertRaises(ValueError): module.validate(*args)

    def test_source_object_image_and_hypothesis_corruption_refuses(self):
        for mode in ('source', 'pin', 'inventory', 'bytes', 'admission', 'unit', 'header', 'image', 'map',
                     'derivative', 'original', 'replacement', 'dependency', 'hypothesis', 'object', 'unchanged_object'):
            args = self.fixture(); build, before, originals, positive, images, mutants = args[1:]
            item = mutants['early_restore']; manifest = item['manifest']
            if mode == 'source': originals['core/inc/types/Event.h'] += b'changed'
            elif mode == 'pin': before['core_commit'] = 'a'*40; build['source'] = copy.deepcopy(before)
            elif mode == 'inventory': before['files']['unknown'] = module.identity(b'unknown'); build['source'] = copy.deepcopy(before)
            elif mode == 'bytes': before['files']['compiler/driver']['bytes'] = True; build['source'] = copy.deepcopy(before)
            elif mode == 'admission': manifest['guest_executed'] = True
            elif mode == 'unit': build['units']['probe'] = ['samd/paired-irq/profile.cpp']
            elif mode == 'header': build['units']['timer'].remove('core/inc/types/Event.h')
            elif mode == 'image': item['elf'] += b'changed'
            elif mode == 'map': images['map'] += b'changed'
            elif mode == 'derivative': item['source'] += b'invented'; manifest['generated_source'] = module.identity(item['source'])
            elif mode == 'original': manifest['original_source']['sha256'] = 'f'*64
            elif mode == 'replacement': manifest['replaced_unit'] = 'timer'
            elif mode == 'dependency': manifest['dependencies'].append('unknown/header')
            elif mode == 'hypothesis': manifest['expected_failure']['status'] = '0x600d'
            elif mode == 'object': item['objects']['profile'] += b'changed'
            else: item['objects']['timer'] += b'changed'; manifest['objects']['timer'] = module.identity(item['objects']['timer'])
            with self.subTest(mode=mode), self.assertRaises(ValueError): module.validate(*args)

    def entrypoint(self, mode=None):
        logs, build, before, originals, positive, images, mutants = self.fixture()
        head = 'a'*40
        env = {'LABWIRED_TIMER_POSITIVE_DIR': '/manufactured/positive',
               'LABWIRED_TIMER_MUTANT_DIR': '/manufactured/mutants',
               'LABWIRED_TIMER_SOURCE_DIR': '/manufactured/samd', 'LABWIRED_TIMER_CORE_DIR': '/manufactured/core',
               'QUALIFICATION_HEAD': head, 'GITHUB_REPOSITORY': 'w1ne/labwired-core',
               'GITHUB_EVENT_NAME': 'pull_request', 'GITHUB_RUN_ID': '123', 'GITHUB_RUN_ATTEMPT': '1'}
        if mode == 'repository': env['GITHUB_REPOSITORY'] = 'unknown/repository'
        elif mode == 'event': env['GITHUB_EVENT_NAME'] = 'push'
        elif mode == 'run': env['GITHUB_RUN_ID'] = '0'
        elif mode == 'attempt': env['GITHUB_RUN_ATTEMPT'] = '0'
        def read(path):
            name = str(path)
            if name.startswith('/manufactured/positive/'):
                if path.name == 'build-provenance.json': return json.dumps(build).encode()
                if path.name == 'source-before.json': return json.dumps(before).encode()
                if path.suffix == '.o': return positive[path.stem]
                return images[path.suffix.removeprefix('.')]
            if name.startswith('/manufactured/mutants/'):
                case = path.parent.name; item = mutants[case]
                if path.name == 'mutation.json': return json.dumps(item['manifest']).encode()
                if path.name == 'mutant.cpp': return item['source']
                if path.name == 'mutant.o': return item['objects'][module.CASES[case][1]]
                return item[path.suffix.removeprefix('.')]
            if name.startswith('/manufactured/'):
                label = name.removeprefix('/manufactured/')
                if mode == 'live_bytes' and label == 'samd/timer-caller/probe.cpp': return b'changed'
                return originals[label]
            if path.name.startswith('timer-refusal-'): return logs[path.stem.removeprefix('timer-refusal-')]
            return b'changed helper' if mode == 'helper' else b'manufactured helper'
        def git(*args):
            if args[0] == '-C':
                if args[-1] == 'HEAD':
                    return 'b'*40 if mode == 'live_pin' else (module.SAMD if args[1].endswith('samd') else module.CORE)
                return ''
            if args[0] == 'show': return 'b'*40 + ' ' + ('c'*40 if mode == 'head' else head)
            return 'd'*40
        def raw_git(args):
            if args[1] == '-C': return originals[args[2].split('/')[-1] + '/' + args[-1].removeprefix('HEAD:')]
            return b'manufactured helper'
        output = mock_open()
        compiler = {label: before['files'][label] for label in module.COMPILER_FILES}
        if mode == 'compiler': compiler['compiler/driver'] = module.identity(b'changed compiler')
        if mode == 'existing': output.side_effect = FileExistsError('existing receipt')
        with patch.dict(module.os.environ, env, clear=True), patch.object(Path, 'read_bytes', read), \
                patch.object(module, 'git', git), patch.object(module.subprocess, 'check_output', raw_git), \
                patch.object(module, 'compiler_snapshot', return_value=compiler), \
                patch.object(Path, 'open', output):
            if mode:
                with self.assertRaises(FileExistsError if mode == 'existing' else ValueError): module.main()
                output.return_value.write.assert_not_called()
                if mode != 'existing': output.assert_not_called()
                return
            module.main()
        output.assert_called_once_with('x')
        receipt = json.loads(output.return_value.write.call_args.args[0])
        self.assertEqual(receipt['head'], head)
        self.assertEqual(receipt['run'], 123)
        self.assertEqual(len(receipt['mutants']), 3)
        self.assertEqual(set(receipt['profiles']), {'default', 'scheduler'})

    def test_manufactured_entrypoint_creates_exclusive_bound_receipt(self):
        self.entrypoint()

    def test_entrypoint_bad_identity_or_existing_receipt_writes_nothing(self):
        for mode in ('repository', 'event', 'run', 'attempt', 'live_pin', 'live_bytes', 'helper', 'head', 'existing', 'compiler'):
            with self.subTest(mode=mode): self.entrypoint(mode)

if __name__ == '__main__':
    unittest.main()
