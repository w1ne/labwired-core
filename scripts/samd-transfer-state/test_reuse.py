# SPDX-License-Identifier: MIT
"""Pure synthetic gate controls, not guest or original artifact replay."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('reuse.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class ReuseControls(unittest.TestCase):
    def setUp(self):
        self.record = dict(schema=1, fixture=gate.FIXTURE, run=7, attempt=1,
                           commit='a' * 40, tree='b' * 40, head='c' * 40)
        self.run = dict(id=7, run_attempt=1, conclusion='success', event='pull_request',
                        path=gate.WORKFLOW, head_sha='c' * 40)
        self.commit = dict(sha='a' * 40, commit={'tree': {'sha': 'b' * 40}},
                           parents=[{'sha': 'd' * 40}, {'sha': 'c' * 40}])

    def allowed(self, changes):
        return gate.admissible(self.record, self.run, self.commit, changes)

    def test_same_snapshot_and_allowed_documentation(self):
        self.assertTrue(self.allowed([]))
        self.assertTrue(self.allowed(sorted(gate.ALLOWED_DOCS)))

    def test_every_non_allowlisted_input_requires_execution(self):
        for path in ['crates/core/src/lib.rs', 'Cargo.lock', '.cargo/config.toml',
                     gate.WORKFLOW, 'scripts/samd-transfer-state/reuse.py',
                     'crates/core/tests/samd_transfer_state_guest.rs',
                     'configs/chips/atsamd51.yaml', 'unrecognized.md']:
            self.assertFalse(self.allowed([path]), path)

    def test_cancelled_failed_wrong_run_and_attempt_rejected(self):
        for field, value in [('conclusion', 'cancelled'), ('conclusion', 'failure'),
                             ('id', 8), ('run_attempt', 2), ('event', 'push'),
                             ('path', 'another.yml'), ('head_sha', 'e' * 40)]:
            original = copy.deepcopy(self.run)
            self.run[field] = value
            self.assertFalse(self.allowed([]), (field, value))
            self.run = original

    def test_changed_fixture_commit_tree_or_parent_rejected(self):
        for field in ['fixture', 'commit', 'tree', 'head', 'schema', 'attempt']:
            old = self.record[field]
            self.record[field] = 'wrong'
            self.assertFalse(self.allowed([]), field)
            self.record[field] = old
        self.commit['parents'].pop()
        self.assertFalse(self.allowed([]))

    def packet(self, negative_marker='0x4', prefix=''):
        raw = io.BytesIO()
        log = (f'SAMD_TRANSFER_STATE case=positive elf_sha256={gate.POSITIVE} status=0x600d steps=1425 bss_clear=true entry=true\n'
               f'SAMD_TRANSFER_STATE case=premature-success elf_sha256={gate.NEGATIVE} status={negative_marker} steps=302 bss_clear=true entry=true\n'
               'test result: ok. 1 passed; 0 failed; 0 ignored\n')
        with zipfile.ZipFile(raw, 'w') as z:
            z.writestr(prefix + 'qualification.json', json.dumps(self.record))
            for name in ['state-guest-default.log', 'state-guest-scheduler.log']:
                z.writestr(name, log)
        return raw.getvalue()

    def test_both_exact_profiles_required(self):
        self.assertEqual(gate.evidence(self.packet()), self.record)
        with self.assertRaises(AssertionError):
            gate.evidence(self.packet(negative_marker='0x600d'))

    def test_unsafe_path_and_missing_provenance_rejected(self):
        with self.assertRaises(AssertionError):
            gate.evidence(self.packet(prefix='../'))
        raw = io.BytesIO()
        with zipfile.ZipFile(raw, 'w') as z:
            z.writestr('qualification-reuse.json', '{}')
        with self.assertRaises(AssertionError):
            gate.evidence(raw.getvalue())


if __name__ == '__main__':
    unittest.main()
