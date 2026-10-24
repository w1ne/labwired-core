#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bound negative receipt producer; not an independent audit or replay."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

SAMD = 'd55214588780d737640536b0a58707cd6d69cfce'
CORE = '2ed19d64ec3359143409a4b0a9758b4aeef3ee7d'
CORE_FILES = ('source/driver-models/Timer.cpp', 'inc/driver-models/Timer.h',
              'inc/driver-models/LowLevelTimer.h', 'inc/core/CodalConfig.h',
              'inc/core/CodalComponent.h', 'inc/core/ErrorNo.h', 'inc/core/codal_target_hal.h',
              'inc/types/Event.h', 'inc/core/CodalCompat.h', 'inc/core/CodalDmesg.h', 'LICENSE')
SAMD_FILES = ('timer-caller/probe.cpp', 'timer-caller/platform_includes.h',
              'timer-caller/build.sh', 'timer-caller/provenance.py', 'timer-caller/test_provenance.py',
              'paired-irq/profile.cpp', 'paired-irq/include/PairedIrqState.h',
              'simulation/include/InterruptMask.h', 'simulation/arm/irq-startup.S',
              'simulation/arm/control.ld')
COMPILER_FILES = {'compiler/driver', 'compiler/libgcc.a', 'compiler/include/stdbool.h'}
FILES = ({'core/' + p for p in CORE_FILES} | {'samd/' + p for p in SAMD_FILES} | COMPILER_FILES)
LEAVE = '    apply(state.leave(saved, context));'
ENTRY = '    require(Mask::read() == 1 && bw_simulation_irq_fault == 0, 0xb01);'
CASES = {
    'unconditional_restore': ('paired-irq/profile.cpp', 'profile', LEAVE,
                             LEAVE + '\n    Mask::restore(0);', '0xb02', 0, 0, 0),
    'early_restore': ('paired-irq/profile.cpp', 'profile', LEAVE,
                      '    Mask::restore(0);\n' + LEAVE, '0xa14', 16, 1, 1),
    'unsupported_event': ('timer-caller/probe.cpp', 'probe', ENTRY,
                          ENTRY + '\n    codal::Event unsupported(42, 7, 0, codal::CREATE_AND_FIRE);',
                          '0xa07', 0, 0, 0),
}

def require(ok, reason):
    if not ok:
        raise ValueError(reason)

def identity(data):
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}

def compiler_snapshot():
    executable = shutil.which('arm-none-eabi-g++')
    require(executable is not None, 'cross compiler missing')
    driver = Path(executable).resolve()
    libgcc = Path(subprocess.check_output([str(driver), '-print-libgcc-file-name']).decode().strip()).resolve()
    include = Path(subprocess.check_output([str(driver), '-print-file-name=include']).decode().strip()).resolve()
    return {label: identity(path.read_bytes()) for label, path in
            [('compiler/driver', driver), ('compiler/libgcc.a', libgcc),
             ('compiler/include/stdbool.h', include / 'stdbool.h')]}

def verify_compiler(after, before):
    require(set(after) == COMPILER_FILES, 'exact recorded compiler components')
    require(after == {label: before['files'][label] for label in COMPILER_FILES},
            'compiler components changed during negative build/execution')

def validate(logs, build, before, originals, positive_objects, positive_images, mutants):
    require(set(logs) == {'default', 'scheduler'}, 'both profiles required')
    require(type(build['schema']) is int and build['schema'] == 1
            and type(before['schema']) is int and before['schema'] == 1, 'positive build/source schema')
    require(build['source'] == before and before['samd_commit'] == SAMD
            and before['core_commit'] == CORE, 'positive source drift/pin')
    require(build['guest_executed'] is False and build['runtime_admitted'] is False,
            'build must not claim execution/admission')
    require(set(before['files']) == FILES and set(originals) == FILES - COMPILER_FILES,
            'complete exact source/compiler inventory required')
    for label, value in before['files'].items():
        require(set(value) == {'bytes', 'sha256'} and type(value['bytes']) is int
                and value['bytes'] > 0 and re.fullmatch('[0-9a-f]{64}', value['sha256']), 'input identity')
        if label in originals:
            require(identity(originals[label]) == value, 'immutable source bytes mismatch')
    require(set(build['units']) == {'timer', 'profile', 'probe'}, 'three actual compilation units')
    for unit, source in [('timer', 'core/source/driver-models/Timer.cpp'),
                          ('profile', 'samd/paired-irq/profile.cpp'), ('probe', 'samd/timer-caller/probe.cpp')]:
        labels = build['units'][unit]
        require(isinstance(labels, list) and len(labels) == len(set(labels))
                and source in labels and set(labels) <= FILES, 'positive unit closure/source')
    require({'core/' + name for name in CORE_FILES if name != 'LICENSE'} <= set(build['units']['timer']),
            'complete unchanged actual Timer header closure')
    require(set(positive_objects) == {'startup', 'timer', 'profile', 'probe'}, 'four positive objects')
    require(set(positive_images) == {'elf', 'map'}, 'positive ELF/map required')
    for field, data in positive_images.items():
        require(bool(data) and identity(data) == build[field], 'positive image drift')
    require(set(mutants) == set(CASES), 'three unique owned cases')
    for case, (path, unit, selector, replacement, status, exception, _, _) in CASES.items():
        item = mutants[case]; manifest = item['manifest']
        require(type(manifest['schema']) is int and manifest['schema'] == 1,
                'mutation schema')
        require(manifest['case'] == case and manifest['samd_commit'] == SAMD,
                'mutation case/source')
        require(manifest['guest_executed'] is False and manifest['runtime_admitted'] is False,
                'mutation must not claim execution/admission')
        original = originals['samd/' + path]
        require(original.count(selector.encode()) == 1, 'unique original mutation site')
        generated = original.replace(selector.encode(), replacement.encode(), 1)
        require(item['source'] == generated and manifest['generated_source'] == identity(generated),
                'exact authored generated derivative required')
        require(manifest['original_source'] == {'label': 'samd/' + path, **identity(original)}
                and manifest['replaced_unit'] == unit, 'original source/unit binding')
        require(manifest['expected_failure'] == {'status': status, 'fault': 0, 'exception': exception},
                'fixed failure hypothesis')
        require(type(manifest['expected_failure']['fault']) is int
                and type(manifest['expected_failure']['exception']) is int, 'typed failure hypothesis')
        closure = (set(build['units'][unit]) - {'samd/' + path}) | {'generated/' + unit}
        require(set(manifest['dependencies']) == closure and len(manifest['dependencies']) == len(closure),
                'exact changed-unit dependency closure')
        require(set(item['objects']) == set(positive_objects) and set(manifest['objects']) == set(positive_objects),
                'complete actual object inventory')
        for name, data in item['objects'].items():
            require(bool(data) and identity(data) == manifest['objects'][name], 'consumed object drift')
            if name != unit:
                require(data == positive_objects[name], 'unchanged linked object drift')
        for field in ('elf', 'map'):
            require(bool(item[field]) and identity(item[field]) == manifest[field], 'mutant image drift')
    results = {}
    for profile, data in logs.items():
        rows = [line for line in data.decode().splitlines() if line.startswith('SAMD_TIMER_REFUSAL')]
        require(len(rows) == 3, 'exactly three complete negative rows')
        seen = {}
        for line in rows:
            match = re.fullmatch(r'SAMD_TIMER_REFUSAL case=([a-z_]+) elf_sha256=([0-9a-f]{64}) '
                                 r'steps=([0-9]+) terminal=64 status=(0x[0-9a-f]+) fault=0 mask=1 '
                                 r'exception=([0-9]+) irq_entries=([0-9]+) milestones=([0-9]+) startup=true', line)
            require(match is not None, 'complete exact rejection witnesses')
            case, digest, steps, status, exception, entries, milestones = match.groups()
            require(case in CASES and case not in seen, 'unknown/duplicate negative case')
            _, _, _, _, expected_status, expected_exception, expected_entries, expected_milestones = CASES[case]
            require(digest == mutants[case]['manifest']['elf']['sha256'] and 0 < int(steps) <= 250000,
                    'actual consumed image/step bound')
            require((status, int(exception), int(entries), int(milestones)) ==
                    (expected_status, expected_exception, expected_entries, expected_milestones),
                    'wrong rejection is not qualification')
            seen[case] = {'steps': int(steps), 'status': status, 'fault': 0, 'mask': 1,
                          'exception': int(exception), 'irq_entries': int(entries),
                          'milestones': int(milestones), 'terminal': 64}
        require(set(seen) == set(CASES), 'all three cases executed')
        results[profile] = {'log_sha256': identity(data)['sha256'], 'cases': seen}
    return results

def git(*args):
    return subprocess.check_output(['git', *args], text=True).strip()

def main():
    positive = Path(os.environ['LABWIRED_TIMER_POSITIVE_DIR'])
    root = Path(os.environ['LABWIRED_TIMER_MUTANT_DIR'])
    build = json.loads((positive / 'build-provenance.json').read_bytes())
    before = json.loads((positive / 'source-before.json').read_bytes())
    originals = {}
    for role, variable, pin, paths in [('samd', 'LABWIRED_TIMER_SOURCE_DIR', SAMD, SAMD_FILES),
                                       ('core', 'LABWIRED_TIMER_CORE_DIR', CORE, CORE_FILES)]:
        checkout = Path(os.environ[variable])
        require(git('-C', str(checkout), 'rev-parse', 'HEAD') == pin, 'live immutable pin')
        require(not git('-C', str(checkout), 'status', '--porcelain', '--untracked-files=no'), 'live clean source')
        for name in paths:
            data = (checkout / name).read_bytes()
            require(data == subprocess.check_output(['git', '-C', str(checkout), 'show', 'HEAD:' + name]),
                    'immutable Git/live source mismatch')
            originals[role + '/' + name] = data
    positive_objects = {name: (positive / (name + '.o')).read_bytes()
                        for name in ('startup', 'timer', 'profile', 'probe')}
    mutants = {}
    for case, (_, unit, *_) in CASES.items():
        directory = root / case
        mutants[case] = {'manifest': json.loads((directory / 'mutation.json').read_bytes()),
                         'source': (directory / 'mutant.cpp').read_bytes(),
                         'elf': (directory / 'control.elf').read_bytes(),
                         'map': (directory / 'control.map').read_bytes(),
                         'objects': {name: (directory / 'mutant.o').read_bytes() if name == unit else data
                                     for name, data in positive_objects.items()}}
    logs = {profile: Path('timer-refusal-' + profile + '.log').read_bytes()
            for profile in ('default', 'scheduler')}
    profiles = validate(logs, build, before, originals, positive_objects,
                        {field: (positive / ('control.' + field)).read_bytes() for field in ('elf', 'map')}, mutants)
    compiler_after = compiler_snapshot()
    verify_compiler(compiler_after, before)
    head = os.environ['QUALIFICATION_HEAD']
    parents = git('show', '-s', '--format=%P', 'HEAD').split()
    require(re.fullmatch('[0-9a-f]{40}', head) and len(parents) == 2 and parents[1] == head, 'merge/head identity')
    require(os.environ['GITHUB_REPOSITORY'] in ('w1ne/labwired-core', 'CrispStrobe/labwired-core')
            and os.environ['GITHUB_EVENT_NAME'] == 'pull_request', 'official qualification context')
    run, attempt = int(os.environ['GITHUB_RUN_ID']), int(os.environ['GITHUB_RUN_ATTEMPT'])
    require(run > 0 and attempt > 0, 'positive official run/attempt')
    helpers = {}
    for name in ('scripts/samd-real-timer-adversaries/build.py', 'scripts/samd-real-timer-adversaries/record.py',
                 'crates/core/tests/samd_real_timer_adversary_guest.rs'):
        data = Path(name).read_bytes()
        require(data == subprocess.check_output(['git', 'show', 'HEAD:' + name]), 'qualification helper drift')
        helpers[name] = identity(data)
    result = {'schema': 1, 'head': head, 'commit': git('rev-parse', 'HEAD'), 'tree': git('rev-parse', 'HEAD^{tree}'),
              'parents': parents, 'run': run, 'attempt': attempt, 'build': build, 'helpers': helpers,
              'compiler_after': compiler_after,
              'mutants': {case: item['manifest'] for case, item in mutants.items()}, 'profiles': profiles,
              'boundary': 'Owned adapter negative guests only; no full runtime/toolchain/hardware qualification'}
    with Path('timer-refusal-qualification.json').open('x') as output:
        output.write(json.dumps(result, indent=2, sort_keys=True) + '\n')

if __name__ == '__main__':
    main()
