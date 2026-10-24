# SPDX-License-Identifier: MIT
"""Derive a public, path-remapped probe input from retained evidence, without execution."""
import argparse
import hashlib
import json
from pathlib import PurePosixPath
import zipfile

REPORT_SHA = 'bb2429fc32505adbb50fc790592cd3a23d5728cdc438e7d2c2c7a7eca2746c1e'
CONFIG_SHA = '1d812a7253fb632cc30537cb34141a1394778c845918172ff5cd36e4953727d8'
REQUEST_SHA = '19efcdc73769fdfdeb51aa215c528bebad59782cbc538f72f4a194326f1f42b1'
COMPILER_SHA = '8b954aa209c87cc599255fc91618e2e7cc41f913d37a1bd233fdd1c13e3274a3'
COMPONENTS = {
    '.': ['codal', '7cf09a3a37c8aca6e100b7265c81e6cdf6129dd2'],
    'libraries/codal-core': ['codal-core', '312ae57e0b31f5b9df07a81e9d846945828e3c5a'],
    'libraries/codal-itsybitsy-m4': ['codal-itsybitsy-m4', '6ecd80ccf1abcc126d06653f178ca03a4d1c6421'],
    'libraries/codal-samd': ['codal-samd', '5bd6b93c219c7e784e885ba2d6812809fb6289a8'],
    'libraries/codal-samd/asf4': ['asf4', '6664673f70d9170b4374a726c5322e9be6b3f237'],
    'libraries/codal-samd/samd-peripherals': ['samd-peripherals', '96563308fc7b97646cbe429953e79cb3405846f0'],
}
CORE_CALLERS = [
    'JACDAC/JACDAC.cpp', 'JACDAC/JDPhysicalLayer.cpp', 'core/CodalDmesg.cpp',
    'core/CodalFiber.cpp', 'core/CodalHeapAllocator.cpp', 'core/codal_default_target_hal.cpp',
    'driver-models/Serial.cpp', 'driver-models/Timer.cpp', 'drivers/MessageBus.cpp',
]
SOURCES = ({'libraries/codal-core/source/' + p for p in CORE_CALLERS} |
           {'libraries/codal-itsybitsy-m4/src/codal_target_hal.cpp'} |
           {'libraries/codal-samd/src/' + p for p in ['DmaInstance.cpp', 'SAMDEIC.cpp', 'ZPin.cpp']} |
           {'pxtapp/' + p for p in ['core---samd/hf2.cpp', 'core---samd/light.cpp',
                                    'mixer---samd/melody.cpp', 'pointers.cpp',
                                    'screen---st7735/panic.cpp', 'screen---st7735/jddisplay.cpp',
                                    'settings/SAMDFlash.cpp', 'settings/RP2040Flash.cpp']})


def sha(data):
    return hashlib.sha256(data).hexdigest()


def closed(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key')
        result[key] = value
    return result


def relative(value):
    path = PurePosixPath(value)
    if not value or path.is_absolute() or '..' in path.parts or str(path) != value:
        raise ValueError('noncanonical relative path')
    return value


def derive(report, config):
    if report['schema'] != 1 or report['completeSourceToBinaryProof'] is not False:
        raise ValueError('unexpected diagnostic boundary')
    if len(report['units']) != 172 or len(report['files']) != 542:
        raise ValueError('unexpected diagnostic inventory')
    if report['compilers']['/usr/bin/arm-none-eabi-g++']['sha256'] != COMPILER_SHA:
        raise ValueError('unexpected compiler')
    units, groups, files = [], [], {}
    for unit in sorted(report['units'], key=lambda u: u['source']):
        source = unit['source']
        if source not in SOURCES:
            continue
        invocation = unit['invocation']
        # Infer, but never publish, the captured host root from the exact source tail.
        if not invocation[-1].endswith('/' + source):
            raise ValueError('source command mismatch')
        root = invocation[-1][:-len(source)-1]
        if invocation[0] != '/usr/bin/arm-none-eabi-g++' or invocation[-6:-4] != ['-M', '-MT'] or invocation[-3] != '-MF':
            raise ValueError('unexpected separate dependency-probe grammar')
        flags = []
        for flag in invocation[1:-6]:
            if root in flag:
                flag = flag.replace(root, '{source}')
            # Original flags sometimes contain /./. Canonicalize only path arguments.
            if flag.startswith('-I{source}/'):
                flag = '-I{source}/' + str(PurePosixPath(flag[len('-I{source}/'):]))
            if '/' in flag and '{source}' not in flag:
                raise ValueError('unmapped host path')
            flags.append(flag)
        if flags not in groups:
            groups.append(flags)
        dependencies = sorted(set(unit['dependencies']))
        for key in dependencies:
            entry = report['files'][key]
            path = relative(entry['path'])
            if key != entry['origin'] + '/' + path or entry['origin'] not in ('build-source', 'toolchain'):
                raise ValueError('unbound dependency')
            files[key] = {k: entry[k] for k in ('origin', 'path', 'bytes', 'sha256')}
        units.append({'source': source, 'sourceSha256': unit['sourceSha256'],
                      'flagsGroup': groups.index(flags), 'dependencies': dependencies,
                      'originalRecipeSha256': unit['recipeSha256'],
                      'originalFlagsSha256': unit['flagsSha256']})
    if {u['source'] for u in units} != SOURCES or len(units) != len(SOURCES):
        raise ValueError('missing or duplicate selected source')
    keys = sorted(files)
    for unit in units:
        unit['dependencies'] = [keys.index(key) for key in unit['dependencies']]
    return {'schema': 1, 'status': 'SEPARATE_PREPROCESSOR_INPUT',
            'originalDiagnosticReportSha256': REPORT_SHA, 'requestSha256': REQUEST_SHA,
            'configurationSha256': CONFIG_SHA, 'configuration': config.decode(),
            'compilerSha256': COMPILER_SHA, 'components': COMPONENTS,
            'flagsGroups': groups, 'units': units, 'files': dict(sorted(files.items())),
            'runtimeAdmission': False, 'runtimeBuildSelection': False,
            'activeCallProof': False,
            'boundary': 'Selected original diagnostic flags with declared source-root remapping and include-path dot normalization; new preprocessing only, not original invocations, retained code, assembly, licence review or execution'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', help='already retained original ZIP; no acquisition')
    args = parser.parse_args()
    try:
        with zipfile.ZipFile(args.archive) as archive:
            data = {}
            for name, digest, maximum in [
                ('diagnostic-dependencies.json', REPORT_SHA, 3_000_000),
                ('generated/build/codal_extra_definitions.h', CONFIG_SHA, 1634),
            ]:
                if archive.namelist().count(name) != 1 or archive.getinfo(name).file_size > maximum:
                    raise ValueError('missing, duplicate or oversized input')
                data[name] = archive.read(name)
                if sha(data[name]) != digest:
                    raise ValueError('original byte binding mismatch')
        result = derive(json.loads(data['diagnostic-dependencies.json'], object_pairs_hook=closed),
                        data['generated/build/codal_extra_definitions.h'])
        print(json.dumps(result, indent=2, sort_keys=True))
    except (ValueError, KeyError, TypeError, OSError, zipfile.BadZipFile):
        parser.exit(2, 'probe input refused: missing, malformed or mismatched retained evidence\n')


if __name__ == '__main__':
    main()
