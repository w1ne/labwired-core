# SPDX-License-Identifier: MIT
"""Pure refusal/parser controls. No compiler, source fetch or firmware execution."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import freeze
import probe


class ProbeControls(unittest.TestCase):
    def test_frozen_input_and_selected_scope(self):
        m = probe.load_input()
        self.assertEqual(len(m['units']), 21)
        self.assertEqual(len(m['files']), 278)
        self.assertEqual(len(m['flagsGroups']), 4)
        keys = sorted(m['files'])
        for u in m['units']:
            self.assertEqual(u['dependencies'], sorted(set(u['dependencies'])))
            self.assertTrue(all(type(i) is int and 0 <= i < len(keys) for i in u['dependencies']))
            self.assertEqual(m['files']['build-source/' + u['source']]['sha256'], u['sourceSha256'])
        raw = Path(probe.__file__).with_name('input.json').read_bytes()
        for forbidden in [b'/home/', b'/mnt/', b'/tmp/', b'_temp/', b'/Users/']:
            self.assertNotIn(forbidden, raw)
        self.assertIs(m['runtimeAdmission'], False)
        self.assertIs(m['runtimeBuildSelection'], False)
        self.assertIs(m['activeCallProof'], False)

    def test_closed_flags_and_mutations(self):
        m = probe.load_input()
        for group in m['flagsGroups']:
            result = probe.flags_for(group, Path('/synthetic/source'))
            self.assertNotIn('{source}', '\n'.join(result))
        group = m['flagsGroups'][0]
        for extra in ['@response', '-fplugin=evil', '-specs=evil', '-o', '-c', '-S',
                      '-I/foreign', '-I{source}/../foreign', '-include', '-E', '-DUNKNOWN=1']:
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                probe.flags_for([*group, extra], Path('/synthetic/source'))
        for bad in [[], [f for f in group if f not in ('-include', '{source}/build/codal_extra_definitions.h')],
                    [*group, '-include', '{source}/build/codal_extra_definitions.h']]:
            with self.assertRaises(ValueError):
                probe.flags_for(bad, Path('/synthetic/source'))

    def test_markers_spelling_boundary_and_refusals(self):
        root = Path('/synthetic/source')
        text = '# 1 "<built-in>"\n# 20 "/synthetic/source/libraries/caller.cpp" 1\n' \
               'void target_disable_irq();\ntarget_disable_irq(); target_enable_irq();\n' \
               '# 50 "/synthetic/source/pxtapp/caller.h" 1\n__disable_irq();\n'
        m = {'files': {'build-source/libraries/caller.cpp': {}, 'build-source/pxtapp/caller.h': {}}}
        rows = probe.observations(text, root, m)
        self.assertEqual([r['line'] for r in rows], [20, 21, 50])
        # Declaration included intentionally: these are spelling observations, not calls.
        self.assertEqual(rows[0]['spellings'], ['target_disable_irq'])
        for bad in [text.replace('libraries/caller.cpp', 'libraries/unknown.cpp'),
                    text.replace('/synthetic/source/libraries/caller.cpp', '/foreign/caller.cpp'),
                    text.replace('<built-in>', '<unknown>'), 'target_disable_irq();\n']:
            with self.assertRaises(ValueError):
                probe.observations(bad, root, m)

    def test_dependency_rule_and_duplicate_json(self):
        root = Path('/synthetic/source')
        self.assertEqual(probe.dependency_keys('probe.o: /synthetic/source/pxtapp/a.cpp \\\n /usr/include/newlib/a.h\n', root),
                         ['build-source/pxtapp/a.cpp', 'toolchain/include/newlib/a.h'])
        for bad in ['wrong.o: /usr/include/a.h', 'probe.o: $(BAD)',
                    'probe.o: /foreign/header.h', 'probe.o: /usr/a.h:extra']:
            with self.assertRaises(ValueError):
                probe.dependency_keys(bad, root)
        with self.assertRaises(ValueError):
            json.loads('{"schema":1,"schema":1}', object_pairs_hook=freeze.closed)

    def test_hosted_refusal_before_any_subprocess(self):
        with patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}), patch.object(probe.subprocess, 'run') as run:
            with self.assertRaises(ValueError):
                probe.stage(Path('/synthetic/source'), Path('/synthetic/request'), probe.load_input())
            with self.assertRaises(ValueError):
                probe.probe(Path('/synthetic/source'), Path('/synthetic/output'), probe.load_input())
            run.assert_not_called()

    def test_original_zip_refusal_and_empty_stdout(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / 'synthetic.zip'
            with zipfile.ZipFile(archive, 'w') as z:
                z.writestr('diagnostic-dependencies.json', '{}')
            result = subprocess.run([sys.executable, '-B', str(Path(freeze.__file__)), str(archive)],
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, '')
            self.assertNotIn(directory, result.stderr)


if __name__ == '__main__':
    unittest.main()
