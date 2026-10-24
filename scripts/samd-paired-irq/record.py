#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Prospective producer controls, not independent or whole-runtime evidence."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

SOURCE_FILES = (
    'paired-irq/profile.cpp', 'paired-irq/include/PairedIrqState.h',
    'paired-irq/arm-probe.cpp', 'simulation/include/InterruptMask.h',
    'simulation/arm/irq-startup.S', 'simulation/arm/control.ld',
)
EXPECTED = [(0, '0x600d', 0, 1, 2, 16, 0), (1, '0x201', 3, 0, 0, 0, 64),
            (2, '0x202', 5, 0, 0, 0, 64), (3, '0x203', 4, 0, 0, 0, 64),
            (4, '0x204', 6, 0, 0, 0, 64)]
ROW = re.compile(
    r'SAMD_PAIRED_IRQ mode=([0-4]) elf_sha256=([0-9a-f]{64}) '
    r'status=(0x[0-9a-f]+) fault=([0-9]+) steps=([0-9]+) '
    r'bss_clear=true data_copy=true entry=true vectors=true '
    r'irq_entries=([0-9]+) stage=([0-9]+) handler_mask=0 '
    r'handler_exception=([0-9]+) final_mask=1 final_exception=0 '
    r'refusal_steps=([0-9]+)'
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


def snapshot(directory, expected):
    require(re.fullmatch('[0-9a-f]{40}', expected), 'immutable source required')
    require(git('-C', str(directory), 'rev-parse', 'HEAD') == expected, 'source pin mismatch')
    require(not git('-C', str(directory), 'status', '--porcelain', '--untracked-files=all'),
            'source must be clean')
    files = {}
    for name in SOURCE_FILES:
        data = (directory / name).read_bytes()
        original = subprocess.check_output(['git', '-C', str(directory), 'show', f'HEAD:{name}'])
        require(data == original, f'working source differs: {name}')
        files[name] = {'sha256': sha(data), 'bytes': len(data)}
    compiler = shutil.which('arm-none-eabi-g++')
    require(compiler is not None, 'cross compiler identity required')
    return {'schema': 1, 'source_commit': expected, 'source_files': files,
            'compiler_sha256': sha(Path(compiler).read_bytes())}


def validate(logs, elfs, metadata, expected_source):
    require(metadata.get('schema') == 1, 'snapshot schema required')
    require(set(logs) == {'default', 'scheduler'}, 'both scheduler profiles required')
    require(set(elfs) == set(range(5)), 'exactly five linked fixtures required')
    require(re.fullmatch('[0-9a-f]{40}', expected_source), 'immutable source required')
    require(metadata['source_commit'] == expected_source, 'source pin mismatch')
    require(set(metadata['source_files']) == set(SOURCE_FILES), 'complete source set required')
    require(re.fullmatch('[0-9a-f]{64}', metadata['compiler_sha256']), 'compiler hash required')
    for entry in metadata['source_files'].values():
        require(re.fullmatch('[0-9a-f]{64}', entry['sha256']) and entry['bytes'] > 0,
                'source identity required')
    results = {}
    for profile, data in logs.items():
        lines = [line for line in data.decode().splitlines() if line.startswith('SAMD_PAIRED_IRQ')]
        require(len(lines) == 5, 'five unique results required')
        rows = []
        for line, expected in zip(lines, EXPECTED):
            match = ROW.fullmatch(line)
            require(match is not None, 'malformed or missing execution witnesses')
            mode, digest, status, fault, steps, entries, stage, exception, refusal = match.groups()
            actual = (int(mode), status, int(fault), int(entries), int(stage), int(exception), int(refusal))
            require(actual == expected, 'wrong, duplicate, reordered or incomplete result')
            require(0 < int(steps) <= 100_000, 'timeout is not qualification')
            require(bool(elfs[int(mode)]) and digest == sha(elfs[int(mode)]), 'linked ELF mismatch')
            rows.append({'mode': int(mode), 'elf_sha256': digest, 'status': status,
                         'fault': int(fault), 'steps': int(steps), 'refusal_steps': int(refusal)})
        results[profile] = {'log_sha256': sha(data), 'results': rows}
    return results


def main():
    expected = os.environ['LABWIRED_PAIRED_QUALIFIED_SOURCE']
    source = Path(os.environ['LABWIRED_PAIRED_SOURCE_DIR'])
    if sys.argv[1:] == ['--snapshot']:
        print(json.dumps(snapshot(source, expected), sort_keys=True, indent=2))
        return
    require(not sys.argv[1:], 'unknown arguments')
    directory = Path(os.environ['LABWIRED_PAIRED_FIXTURE_DIR'])
    before = (directory / 'source-before.json').read_bytes()
    after = (directory / 'source-after.json').read_bytes()
    require(before == after, 'compiler/source changed during build')
    metadata = json.loads(before)
    require(metadata == snapshot(source, expected), 'compiler/source changed before receipt')
    headers = git('cat-file', '-p', 'HEAD').split('\n\n', 1)[0]
    parents = [line[7:] for line in headers.splitlines() if line.startswith('parent ')]
    head = os.environ['QUALIFICATION_HEAD']
    require(len(parents) == 2 and all(re.fullmatch('[0-9a-f]{40}', p) for p in parents)
            and parents[1] == head, 'actual qualification merge/head binding required')
    logs = {profile: Path(f'paired-guest-{profile}.log').read_bytes()
            for profile in ('default', 'scheduler')}
    elfs = {mode: (directory / str(mode) / 'control.elf').read_bytes() for mode in range(5)}
    record = {'schema': 1, 'head': head, 'commit': git('rev-parse', 'HEAD'),
              'tree': git('rev-parse', 'HEAD^{tree}'), 'parents': parents,
              'run': int(os.environ['GITHUB_RUN_ID']), 'attempt': int(os.environ['GITHUB_RUN_ATTEMPT']),
              'source': metadata, 'profiles': validate(logs, elfs, metadata, expected),
              'boundary': 'Authored paired-ABI controls only; not real fibers, peripherals or runtime admission'}
    require(record['run'] > 0 and record['attempt'] > 0, 'official run identity required')
    Path('paired-qualification.json').write_text(json.dumps(record, indent=2) + '\n')


if __name__ == '__main__':
    main()
