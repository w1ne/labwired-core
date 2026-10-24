#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bind this fresh authored PRIMASK execution; this is not a reuse admission gate."""
import hashlib
import json
import os
import pathlib
import re
import subprocess


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


def actual_parents():
    # Revision traversal hides parents at a shallow boundary. Object headers do
    # not: retain the actual commit's parents without weakening source binding.
    headers = git('cat-file', '-p', 'HEAD').split('\n\n', 1)[0]
    parents = [line[7:] for line in headers.splitlines() if line.startswith('parent ')]
    assert all(re.fullmatch('[0-9a-f]{40}', parent) for parent in parents)
    return parents


record = {
    'schema': 1,
    'commit': git('rev-parse', 'HEAD'),
    'tree': git('rev-parse', 'HEAD^{tree}'),
    'parents': actual_parents(),
    'head': os.environ['QUALIFICATION_HEAD'],
    'run': int(os.environ['GITHUB_RUN_ID']),
    'attempt': int(os.environ['GITHUB_RUN_ATTEMPT']),
    'fixture': git('-C', 'owned-mask-source', 'rev-parse', 'HEAD'),
    'logs': {},
}
assert record['fixture'] == 'e15482fd6b537ffd208919c31d568c0bea14d47a'
assert len(record['parents']) == 2 and record['parents'][1] == record['head']
pattern = re.compile(r'^SAMD_INTERRUPT_MASK case=(positive|restore-enabled) elf_sha256=([0-9a-f]{64}) status=(0x600d|0x3) steps=([0-9]+) bss_clear=true entry=true$', re.M)
directory = pathlib.Path(os.environ['RUNNER_TEMP']) / 'mask-guests'
for profile in ['default', 'scheduler']:
    name = f'mask-guest-{profile}.log'
    data = pathlib.Path(name).read_bytes()
    results = pattern.findall(data.decode())
    assert len(results) == 2
    assert [(r[0], r[2]) for r in results] == [('positive', '0x600d'), ('restore-enabled', '0x3')]
    for case, digest, _, steps in results:
        assert 0 < int(steps) <= 100_000
        assert digest == hashlib.sha256((directory / case / 'control.elf').read_bytes()).hexdigest()
    record['logs'][name] = {'sha256': hashlib.sha256(data).hexdigest(), 'results': results}
pathlib.Path('mask-qualification.json').write_text(json.dumps(record, indent=2) + '\n')
