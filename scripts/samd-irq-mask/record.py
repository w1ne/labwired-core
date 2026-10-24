#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Record fresh source-bound IRQ diagnostic evidence; no reuse admission."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


headers = git('cat-file', '-p', 'HEAD').split('\n\n', 1)[0]
parents = [line[7:] for line in headers.splitlines() if line.startswith('parent ')]
assert len(parents) == 2 and all(re.fullmatch('[0-9a-f]{40}', p) for p in parents)
assert parents[1] == os.environ['QUALIFICATION_HEAD']
record = {
    'schema': 1, 'commit': git('rev-parse', 'HEAD'),
    'tree': git('rev-parse', 'HEAD^{tree}'), 'parents': parents,
    'head': os.environ['QUALIFICATION_HEAD'],
    'run': int(os.environ['GITHUB_RUN_ID']), 'attempt': int(os.environ['GITHUB_RUN_ATTEMPT']),
    'fixture': git('-C', 'owned-irq-source', 'rev-parse', 'HEAD'), 'logs': {},
}
assert record['fixture'] == '2a991760339fe89f5272f923baf89dc18fd434d0'
pattern = re.compile(r'^SAMD_IRQ_MASK case=(positive|restore-enabled) elf_sha256=([0-9a-f]{64}) status=(0x600d|0x6) steps=([0-9]+) bss_clear=true entry=true vectors=true irq_count=1 stage=([12]) mask=0 exception=16 irq_entry=true irq_return=true$', re.M)
directory = Path(os.environ['RUNNER_TEMP']) / 'irq-guests'
for profile in ['default', 'scheduler']:
    name = f'irq-guest-{profile}.log'
    data = Path(name).read_bytes()
    results = pattern.findall(data.decode())
    assert len(results) == 2
    assert [(r[0], r[2], r[4]) for r in results] == [('positive', '0x600d', '2'), ('restore-enabled', '0x6', '1')]
    for case, digest, _, steps, _ in results:
        assert 0 < int(steps) <= 100_000
        assert digest == hashlib.sha256((directory / case / 'control.elf').read_bytes()).hexdigest()
    record['logs'][name] = {'sha256': hashlib.sha256(data).hexdigest(), 'results': results}
Path('irq-qualification.json').write_text(json.dumps(record, indent=2) + '\n')
