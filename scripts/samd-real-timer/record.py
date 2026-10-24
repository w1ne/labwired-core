#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Prospective result producer; manufactured controls are not guest evidence."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

CORE_PIN = '2ed19d64ec3359143409a4b0a9758b4aeef3ee7d'
CORE_FILES = ('source/driver-models/Timer.cpp', 'inc/driver-models/Timer.h',
              'inc/driver-models/LowLevelTimer.h', 'inc/core/CodalConfig.h',
              'inc/core/CodalComponent.h', 'inc/core/ErrorNo.h', 'inc/core/codal_target_hal.h',
              'inc/types/Event.h', 'inc/core/CodalCompat.h', 'inc/core/CodalDmesg.h', 'LICENSE')
SAMD_FILES = ('timer-caller/probe.cpp', 'timer-caller/platform_includes.h',
              'timer-caller/build.sh', 'timer-caller/provenance.py', 'timer-caller/test_provenance.py',
              'paired-irq/profile.cpp', 'paired-irq/include/PairedIrqState.h',
              'simulation/include/InterruptMask.h', 'simulation/arm/irq-startup.S',
              'simulation/arm/control.ld')
FILES = ({'core/' + name for name in CORE_FILES} | {'samd/' + name for name in SAMD_FILES}
         | {'compiler/driver', 'compiler/libgcc.a', 'compiler/include/stdbool.h'})
ROW = re.compile(
    r'SAMD_REAL_TIMER elf_sha256=([0-9a-f]{64}) source=([0-9a-f]{40}) steps=([0-9]+) '
    r'bss_clear=true data_copy=true vectors=true milestones=6 irq_entries=2 irq_returns=2 '
    r'handler_mask=0 handler_exception=16 paired_closed=true fault=0 status=0x600d '
    r'final_mask=1 final_exception=0'
)


def require(value, message):
    if not value:
        raise ValueError(message)


def record(data):
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}


def validate(logs, elf, mapping, build, before, expected):
    require(re.fullmatch('[0-9a-f]{40}', expected), 'immutable selected source required')
    require(set(logs) == {'default', 'scheduler'}, 'both scheduler profiles required')
    require(type(build.get('schema')) is int and build['schema'] == 1, 'build schema')
    require(build.get('guest_executed') is False and build.get('runtime_admitted') is False,
            'build must not claim execution or admission')
    source = build['source']
    require(source == before, 'source changed during build')
    require(type(source.get('schema')) is int and source['schema'] == 1 and source['samd_commit'] == expected
            and source['core_commit'] == CORE_PIN, 'source pin mismatch')
    require(set(source['files']) == FILES, 'complete source/compiler inventory')
    for item in source['files'].values():
        require(set(item) == {'bytes', 'sha256'} and type(item['bytes']) is int and item['bytes'] > 0
                and re.fullmatch('[0-9a-f]{64}', item['sha256']), 'source identity required')
    require(bool(elf) and build['elf'] == record(elf), 'consumed linked ELF mismatch')
    require(bool(mapping) and build['map'] == record(mapping), 'linked map mismatch')
    units = build['units']
    require(set(units) == {'timer', 'profile', 'probe'}, 'three actual compilation units required')
    for unit, own_source in [('timer', 'core/source/driver-models/Timer.cpp'),
                             ('profile', 'samd/paired-irq/profile.cpp'),
                             ('probe', 'samd/timer-caller/probe.cpp')]:
        require(isinstance(units[unit], list) and len(units[unit]) == len(set(units[unit]))
                and own_source in units[unit] and set(units[unit]) <= FILES,
                'wrong or unbound unit dependencies')
    require({'core/' + name for name in CORE_FILES if name != 'LICENSE'} <= set(units['timer']),
            'actual Timer header closure missing')
    results = {}
    for profile, data in logs.items():
        lines = [line for line in data.decode().splitlines() if line.startswith('SAMD_REAL_TIMER')]
        require(len(lines) == 1, 'one unique complete Timer result required')
        match = ROW.fullmatch(lines[0])
        require(match is not None, 'missing or incorrect execution witnesses')
        digest, pin, steps = match.groups()
        require(digest == record(elf)['sha256'] and pin == expected, 'guest source/ELF mismatch')
        require(0 < int(steps) <= 250000, 'timeout is not qualification')
        results[profile] = {'log_sha256': record(data)['sha256'], 'steps': int(steps),
                            'status': '0x600d', 'fault': 0, 'milestones': 6, 'irq_entries': 2, 'irq_returns': 2}
    return results


def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()


def main():
    expected = os.environ['LABWIRED_TIMER_QUALIFIED_SOURCE']
    directory = Path(os.environ['LABWIRED_TIMER_FIXTURE_DIR'])
    source_root = Path(os.environ['LABWIRED_TIMER_SOURCE_DIR'])
    core_root = Path(os.environ['LABWIRED_TIMER_CORE_DIR'])
    build = json.loads((directory / 'build-provenance.json').read_bytes())
    before = json.loads((directory / 'source-before.json').read_bytes())
    logs = {profile: Path('timer-guest-' + profile + '.log').read_bytes()
            for profile in ('default', 'scheduler')}
    profiles = validate(logs, (directory / 'control.elf').read_bytes(),
                        (directory / 'control.map').read_bytes(), build, before, expected)
    for role, root, pin, names in [('samd', source_root, expected, SAMD_FILES),
                                   ('core', core_root, CORE_PIN, CORE_FILES)]:
        require(git('-C', str(root), 'rev-parse', 'HEAD') == pin, 'live immutable source pin')
        for name in names:
            data = (root / name).read_bytes()
            original = subprocess.check_output(['git', '-C', str(root), 'show', 'HEAD:' + name])
            require(data == original and record(data) == before['files'][role + '/' + name],
                    'live source drift before receipt')
    head = os.environ['QUALIFICATION_HEAD']
    parents = git('show', '-s', '--format=%P', 'HEAD').split()
    require(re.fullmatch('[0-9a-f]{40}', head) and len(parents) == 2 and parents[1] == head,
            'actual qualification merge/head binding')
    require(os.environ['GITHUB_REPOSITORY'] in ('w1ne/labwired-core', 'CrispStrobe/labwired-core')
            and os.environ['GITHUB_EVENT_NAME'] == 'pull_request', 'official qualification context')
    run, attempt = int(os.environ['GITHUB_RUN_ID']), int(os.environ['GITHUB_RUN_ATTEMPT'])
    require(run > 0 and attempt > 0, 'official run identity')
    result = {'schema': 1, 'head': head, 'commit': git('rev-parse', 'HEAD'),
              'tree': git('rev-parse', 'HEAD^{tree}'), 'parents': parents,
              'run': run, 'attempt': attempt, 'build': build, 'profiles': profiles,
              'boundary': 'Actual Timer caller with explicit diagnostic adapters; not full runtime admission'}
    with Path('timer-qualification.json').open('x') as output:
        output.write(json.dumps(result, indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    main()
