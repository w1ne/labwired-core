# SPDX-License-Identifier: MIT
"""Hosted source staging and bounded -E diagnostics; never build or execute firmware."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import tempfile

from freeze import COMPONENTS, SOURCES, CONFIG_SHA, REQUEST_SHA, COMPILER_SHA, closed, relative, sha

INPUT_SHA = 'f504376d05fec3c5963a7f01af035688586f85bd6f6e0bfcfc74047e308165e0'
COMPILER = '/usr/bin/arm-none-eabi-g++'
SYMBOLS = re.compile(r'\b(?:target_disable_irq|target_enable_irq|__disable_irq|__enable_irq)\b')
MARKER = re.compile(r'^#\s+(\d+)\s+("(?:[^"\\]|\\.)*")(?:\s+\d+)*\s*$')
MACROS = {'SAMD51', 'DEVICE_HEAP_ALLOCATOR', 'DEVICE_USB', 'DEVICE_WEBUSB',
          'CODAL_DEBUG', 'CODAL_DEBUG_DISABLED', 'SCHEDULER_TICK_PERIOD_US',
          'DATASTREAM_MAXIMUM_BUFFERS', 'CODAL', 'PICO_BOARD', 'ARCADE_MBIT_CODAL', 'USE_RGB444'}


def load_input():
    raw = Path(__file__).with_name('input.json').read_bytes()
    if sha(raw) != INPUT_SHA:
        raise ValueError('frozen input identity mismatch')
    manifest = json.loads(raw, object_pairs_hook=closed)
    if manifest['components'] != COMPONENTS or {u['source'] for u in manifest['units']} != SOURCES:
        raise ValueError('frozen source selection mismatch')
    if len(manifest['units']) != 21 or sha(manifest['configuration'].encode()) != CONFIG_SHA:
        raise ValueError('frozen configuration mismatch')
    for key in ('runtimeAdmission', 'runtimeBuildSelection', 'activeCallProof'):
        if manifest[key] is not False:
            raise ValueError('invalid admission claim')
    for group in manifest['flagsGroups']:
        flags_for(group, Path('/synthetic/source'))
    return manifest


def flags_for(group, root):
    # Closed grammar in addition to byte-bound manifest: no plugins, response
    # files, arbitrary outputs, implicit specs, wrappers or linker execution.
    allowed = {
        '-gno-variable-location-views', '-ffile-prefix-map={source}=.',
        '-Wno-expansion-to-defined', '-mcpu=cortex-m4', '-mthumb', '-mfloat-abi=softfp',
        '-mfpu=fpv4-sp-d16', '-fno-exceptions', '-fno-unwind-tables', '-ffunction-sections',
        '-fdata-sections', '-Wall', '-Wextra', '-Wno-unused-parameter', '-std=c++11',
        '-fwrapv', '-fno-rtti', '-fno-threadsafe-statics', '-Wl,--gc-sections',
        '-Wl,--sort-common', '-Wl,--sort-section=alignment', '-Wno-array-bounds',
        '-DCODAL_VERSION="-HEAD-g7cf09a3"', '-O2', '-g', '-DNDEBUG', '-DSAMDX1',
        '-D__SAMD51J19A__', '-DITSYBITSY_M4_EXPORTS',
    }
    result, forced = [], 0
    for i, flag in enumerate(group):
        if flag.startswith('-I{source}/'):
            relative(flag[len('-I{source}/'):])
        elif flag == '-include':
            if i + 1 >= len(group) or group[i+1] != '{source}/build/codal_extra_definitions.h':
                raise ValueError('unexpected forced include')
            forced += 1
        elif flag == '{source}/build/codal_extra_definitions.h':
            if i == 0 or group[i-1] != '-include':
                raise ValueError('orphan include path')
        elif flag not in allowed:
            raise ValueError('unsupported preprocessing flag')
        result.append(flag.replace('{source}', str(root)))
    if forced != 1:
        raise ValueError('missing or repeated forced configuration')
    return result


def hosted():
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise ValueError('compiler and staging diagnostic requires hosted execution')


def git(directory, *args):
    return subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'gc.auto=0',
                           '-C', str(directory), *args], check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, timeout=180).stdout


def stage(root, request_path, manifest):
    hosted()
    if root.exists():
        raise ValueError('source staging directory must be new')
    root.mkdir(parents=True)
    # No recursive submodules/build.py/CMake. Only these six reviewed public Git inputs.
    for prefix, (repository, commit) in COMPONENTS.items():
        directory = root / prefix
        directory.mkdir(parents=True, exist_ok=True)
        git(directory, 'init')
        git(directory, 'fetch', '--no-tags', '--depth=1', '--recurse-submodules=no',
            'https://github.com/lancaster-university/' + repository + '.git', commit)
        git(directory, '-c', 'submodule.recurse=false', 'checkout', '--detach', 'FETCH_HEAD')
        if git(directory, 'rev-parse', 'HEAD').decode().strip() != commit:
            raise ValueError('staged commit mismatch')
    raw = request_path.read_bytes()
    if sha(raw) != REQUEST_SHA:
        raise ValueError('regenerated request differs from original bytes')
    request = json.loads(raw, object_pairs_hook=closed)
    for key, body in request['replaceFiles'].items():
        if key not in ('/codal.json', '/functions.json') and not key.startswith('/pxtapp/'):
            raise ValueError('unexpected request namespace')
        name = relative(key[1:])
        path = root / name
        if path.exists() or path.is_symlink():
            raise ValueError('request would overwrite a staged input')
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as output:
            output.write(body.encode())
    (root / 'build').mkdir()
    with (root / 'build/codal_extra_definitions.h').open('xb') as output:
        output.write(manifest['configuration'].encode())
    print('Six pinned public source inputs and byte-identical request/configuration staged; no build')


def identity(path, maximum=4*1024*1024):
    if not path.is_file() or path.stat().st_size > maximum:
        raise ValueError('missing or oversized input')
    raw = path.read_bytes()
    return {'bytes': len(raw), 'sha256': sha(raw)}


def verify_files(root, manifest):
    for row in manifest['files'].values():
        base = root if row['origin'] == 'build-source' else Path('/usr')
        path = base / relative(row['path'])
        # Toolchain packages have reviewed symlink aliases; bind their resolved
        # bytes within /usr. Source aliases/symlinks are not admitted.
        if row['origin'] == 'build-source' and any(p.is_symlink() for p in [path, *path.parents] if p != root.parent):
            raise ValueError('source dependency symlink')
        if not path.resolve().is_relative_to(base.resolve()):
            raise ValueError('input escapes its reviewed root')
        if identity(path) != {k: row[k] for k in ('bytes', 'sha256')}:
            raise ValueError('dependency byte identity mismatch')
    if identity(Path(COMPILER), 64*1024*1024)['sha256'] != COMPILER_SHA:
        raise ValueError('compiler byte identity mismatch')


def name_for(value, root):
    if value.startswith('<') and value.endswith('>'):
        if value not in ('<built-in>', '<command-line>'):
            raise ValueError('unknown pseudo input')
        return value
    path = Path(value)
    if not path.is_absolute():
        path = root / 'build' / path
    path = path.resolve()
    if path.is_relative_to(root):
        return 'build-source/' + path.relative_to(root).as_posix()
    if path.is_relative_to(Path('/usr')):
        return 'toolchain/' + path.relative_to(Path('/usr')).as_posix()
    raise ValueError('preprocessor reported an unbound input root')


def observations(text, root, manifest):
    name, number, rows = None, 0, []
    for line in text.splitlines():
        marker = MARKER.fullmatch(line)
        if marker:
            number = int(marker[1])
            name = name_for(json.loads(marker[2]), root)
            continue
        hits = SYMBOLS.findall(line)
        if hits:
            if name not in manifest['files']:
                raise ValueError('IRQ spelling in an unbound source input')
            rows.append({'input': name, 'line': number, 'spellings': hits})
        number += 1
    return rows


def limited_files():
    resource.setrlimit(resource.RLIMIT_FSIZE, (32*1024*1024, 32*1024*1024))


def run_preprocessor(command, cwd):
    # Regular owned temporary files bound memory and child output independently;
    # no raw vendor-expanded text or host paths are uploaded or printed.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        process = subprocess.run(command, cwd=cwd, stdout=stdout, stderr=stderr,
                                 timeout=60, preexec_fn=limited_files, check=False)
        stdout.seek(0); stderr.seek(0)
        data, errors = stdout.read(32*1024*1024+1), stderr.read(32*1024*1024+1)
    if process.returncode or len(data) > 32*1024*1024 or len(errors) > 32*1024*1024:
        raise ValueError('bounded preprocessor refused')
    return data, errors


def dependency_keys(text, root):
    # GCC's observed makefile grammar, with only one authored probe.o target.
    import shlex
    text = re.sub(r'\\\r?\n', ' ', text)
    if text.count(':') != 1 or '$' in text:
        raise ValueError('unexpected dependency rule')
    target, body = text.split(':')
    if target.strip() != 'probe.o':
        raise ValueError('unexpected dependency target')
    return sorted({name_for(p, root) for p in shlex.split(body)})


def probe(root, out, manifest):
    hosted()
    if out.exists():
        raise ValueError('probe output directory must be new')
    out.mkdir(parents=True)
    result = {'schema': 1, 'status': 'INCOMPLETE', 'inputSha256': INPUT_SHA,
              'units': [], 'runtimeAdmission': False, 'runtimeBuildSelection': False,
              'activeCallProof': False,
              'boundary': 'Separate -E diagnostics with frozen remapped flags; spelling observations include declarations/definitions/string tokens and are not calls, retention, balance, assembly, notices, runtime admission or guest evidence'}
    try:
        verify_files(root, manifest)
        keys = sorted(manifest['files'])
        for unit in manifest['units']:
            source = root / unit['source']
            if identity(source)['sha256'] != unit['sourceSha256']:
                raise ValueError('source identity mismatch')
            flags = flags_for(manifest['flagsGroups'][unit['flagsGroup']], root)
            with tempfile.TemporaryDirectory() as directory:
                deps = Path(directory) / 'probe.d'
                data, errors = run_preprocessor(
                    [COMPILER, *flags, '-E', '-MD', '-MT', 'probe.o', '-MF', str(deps), str(source)], root / 'build')
                actual = dependency_keys(deps.read_text(), root)
                expected = sorted(keys[i] for i in unit['dependencies'])
                if actual != expected:
                    raise ValueError('effective dependency set differs from frozen diagnostic')
            macros, macro_errors = run_preprocessor([COMPILER, *flags, '-E', '-dM', str(source)], root / 'build')
            selected = {}
            for line in macros.decode().splitlines():
                match = re.fullmatch(r'#define ([A-Za-z_][A-Za-z_0-9]*)(?: (.*))?', line)
                if match and match[1] in MACROS:
                    selected[match[1]] = match[2] or ''
            result['units'].append({'source': unit['source'], 'sourceSha256': unit['sourceSha256'],
                                    'preprocessedSha256': sha(data), 'stderrSha256': sha(errors),
                                    'macroOutputSha256': sha(macros), 'macroStderrSha256': sha(macro_errors),
                                    'dependenciesMatched': True, 'selectedMacros': selected,
                                    'spellingObservations': observations(data.decode(), root, manifest)})
            print('Separate preprocessing completed: ' + unit['source'])
        verify_files(root, manifest)
        result['status'] = 'SELECTED_PREPROCESSOR_PASS'
    finally:
        with (out / 'result.json').open('x') as output:
            json.dump(result, output, indent=2, sort_keys=True)
            output.write('\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['stage', 'probe'])
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--request', type=Path)
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    try:
        manifest = load_input()
        root = args.root.resolve()
        if args.mode == 'stage':
            if args.request is None:
                raise ValueError('request required')
            stage(root, args.request, manifest)
        else:
            if args.out is None:
                raise ValueError('new output required')
            probe(root, args.out.resolve(), manifest)
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError):
        parser.exit(2, 'separate preprocessing refused; no runtime or admission claim\n')


if __name__ == '__main__':
    main()
