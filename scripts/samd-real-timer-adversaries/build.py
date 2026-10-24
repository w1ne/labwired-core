#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build diagnostic mutants of owned adapters; upstream Timer remains unchanged."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import subprocess

PIN = 'd55214588780d737640536b0a58707cd6d69cfce'
CASES = {
    'unconditional_restore': ('paired-irq/profile.cpp',
        '    apply(state.leave(saved, context));',
        '    apply(state.leave(saved, context));\n    Mask::restore(0);',
        {'status': '0xb02', 'fault': 0, 'exception': 0}),
    'early_restore': ('paired-irq/profile.cpp',
        '    apply(state.leave(saved, context));',
        '    Mask::restore(0);\n    apply(state.leave(saved, context));',
        {'status': '0xa14', 'fault': 0, 'exception': 16}),
    'unsupported_event': ('timer-caller/probe.cpp',
        '    require(Mask::read() == 1 && bw_simulation_irq_fault == 0, 0xb01);',
        '    require(Mask::read() == 1 && bw_simulation_irq_fault == 0, 0xb01);\n'
        '    codal::Event unsupported(42, 7, 0, codal::CREATE_AND_FIRE);',
        {'status': '0xa07', 'fault': 0, 'exception': 0}),
}

def identity(data):
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}

def mutate(data, case):
    _, selector, replacement, _ = CASES[case]
    original = selector.encode()
    if data.count(original) != 1:
        raise ValueError('one exact owned-adapter mutation site required')
    result = data.replace(original, replacement.encode(), 1)
    if result == data:
        raise ValueError('mutation must change source bytes')
    return result

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('samd', type=Path)
    parser.add_argument('core', type=Path)
    parser.add_argument('positive', type=Path)
    parser.add_argument('out', type=Path)
    args = parser.parse_args()
    samd, core, positive, out = [p.resolve() for p in (args.samd, args.core, args.positive, args.out)]
    def git(*parts):
        return subprocess.check_output(['git', '-C', str(samd), *parts])
    if git('rev-parse', 'HEAD').decode().strip() != PIN or git('status', '--porcelain', '--untracked-files=no').strip():
        raise ValueError('clean immutable owned source required')
    provenance = json.loads((positive / 'build-provenance.json').read_bytes())
    if provenance['source']['samd_commit'] != PIN:
        raise ValueError('positive source mismatch')
    if provenance['guest_executed'] is not False or provenance['runtime_admitted'] is not False:
        raise ValueError('build provenance must not claim execution/admission')
    core_pin = '2ed19d64ec3359143409a4b0a9758b4aeef3ee7d'
    if subprocess.check_output(['git', '-C', str(core), 'rev-parse', 'HEAD']).decode().strip() != core_pin:
        raise ValueError('core pin mismatch')
    if subprocess.check_output(['git', '-C', str(core), 'status', '--porcelain', '--untracked-files=no']).strip():
        raise ValueError('core tracked source dirty')
    compiler = Path(shutil.which('arm-none-eabi-g++')).resolve()
    libgcc = Path(subprocess.check_output([str(compiler), '-print-libgcc-file-name']).decode().strip()).resolve()
    compiler_include = Path(subprocess.check_output([str(compiler), '-print-file-name=include']).decode().strip()).resolve()
    bound = {}
    for label, expected in provenance['source']['files'].items():
        if label.startswith('samd/'):
            path = samd / label.removeprefix('samd/')
        elif label.startswith('core/'):
            path = core / label.removeprefix('core/')
        else:
            path = {'compiler/driver': compiler, 'compiler/libgcc.a': libgcc,
                    'compiler/include/stdbool.h': compiler_include / 'stdbool.h'}[label]
        if identity(path.read_bytes()) != expected:
            raise ValueError('bound input drift')
        bound[path.resolve()] = label
    for field, name in [('elf', 'control.elf'), ('map', 'control.map')]:
        if identity((positive / name).read_bytes()) != provenance[field]:
            raise ValueError('positive build bytes mismatch')
    includes = ['-I', str(samd / 'timer-caller'), '-isystem', str(core / 'inc/core'),
                '-isystem', str(core / 'inc/driver-models'), '-isystem', str(core / 'inc/types'),
                '-I', str(samd / 'paired-irq/include'), '-I', str(samd / 'simulation/include')]
    flags = ['-mcpu=cortex-m4', '-mthumb', '-std=c++11', '-O0', '-ffreestanding',
             '-ffunction-sections', '-fdata-sections', '-fno-builtin', '-fno-exceptions',
             '-fno-rtti', '-fno-unwind-tables', '-fno-asynchronous-unwind-tables',
             '-DBW_SIMULATION_STRICT_PAIRED_IRQ=1', '-DBW_SIMULATION_TIMER_CALLER=1']
    for case, (path, _, _, expected) in CASES.items():
        original = (samd / path).read_bytes()
        if original != git('show', 'HEAD:' + path):
            raise ValueError('owned source drift')
        if identity(original) != provenance['source']['files']['samd/' + path]:
            raise ValueError('owned source differs from positive manifest')
        directory = out / case
        directory.mkdir(parents=True, exist_ok=False)
        generated = mutate(original, case)
        source = directory / 'mutant.cpp'
        source.write_bytes(generated)
        unit = 'profile' if path.endswith('profile.cpp') else 'probe'
        subprocess.run(['arm-none-eabi-g++', *flags, *includes, '-Wall', '-MD', '-MF',
                        str(directory / 'mutant.d'), '-c', str(source), '-o',
                        str(directory / 'mutant.o')], check=True)
        dependency = (directory / 'mutant.d').read_text().replace('\\\n', '')
        rules = [line for line in dependency.splitlines() if line.strip()]
        if len(rules) != 1 or ':' not in rules[0]:
            raise ValueError('one complete mutant dependency rule required')
        observed = set()
        for name in shlex.split(rules[0].split(':', 1)[1]):
            dependency_path = Path(name).resolve()
            if dependency_path == source:
                observed.add('generated/' + unit)
            else:
                if dependency_path not in bound:
                    raise ValueError('unbound mutant compiler dependency')
                label = bound[dependency_path]
                if identity(dependency_path.read_bytes()) != provenance['source']['files'][label]:
                    raise ValueError('mutant dependency drift')
                observed.add(label)
        expected_closure = set(provenance['units'][unit]) - {'samd/' + path}
        expected_closure.add('generated/' + unit)
        if observed != expected_closure:
            raise ValueError('mutant unit header closure differs from selected unit')
        objects = {name: positive / (name + '.o') for name in ('startup', 'timer', 'profile', 'probe')}
        objects[unit] = directory / 'mutant.o'
        subprocess.run(['arm-none-eabi-g++', '-mcpu=cortex-m4', '-mthumb', '-nostdlib', '-nostartfiles',
                        '-Wl,--gc-sections,--no-undefined,-Map=' + str(directory / 'control.map'),
                        '-T', str(samd / 'simulation/arm/control.ld'),
                        *(str(p) for p in objects.values()), '-lgcc', '-o',
                        str(directory / 'control.elf')], check=True)
        if subprocess.check_output(['arm-none-eabi-nm', '-u', str(directory / 'control.elf')]).strip():
            raise ValueError('unresolved mutant symbols')
        manifest = {'schema': 1, 'case': case, 'samd_commit': PIN,
                    'original_source': {'label': 'samd/' + path, **identity(original)},
                    'generated_source': identity(generated), 'replaced_unit': unit,
                    'dependencies': sorted(observed),
                    'objects': {name: identity(p.read_bytes()) for name, p in objects.items()},
                    'elf': identity((directory / 'control.elf').read_bytes()),
                    'map': identity((directory / 'control.map').read_bytes()),
                    'expected_failure': expected, 'guest_executed': False,
                    'runtime_admitted': False,
                    'boundary': 'Owned adapter mutant build only; guest rejection unobserved'}
        with (directory / 'mutation.json').open('x') as output:
            json.dump(manifest, output, indent=2, sort_keys=True)

if __name__ == '__main__':
    main()
