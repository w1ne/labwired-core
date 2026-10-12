#!/usr/bin/env python3
"""Prepare matching optional RV1103 Linux modules; guest insertion is a separate gate.

Python standard library only. This does not alter the strict base boot fixture.
"""
import argparse
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import selectors
import shutil
import signal
import stat
import struct
import subprocess
import tarfile
import tempfile
import time
import urllib.request

spec = importlib.util.spec_from_file_location('rv1103_base_prepare', Path(__file__).with_name('prepare-rv1103-linux.py'))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

RELEASE = '6.18.56-0-lts'
HEADER_ROOT = 'usr/src/linux-headers-' + RELEASE
VERMAGIC = RELEASE + ' SMP mod_unload modversions ARMv7 p2v8 '
SOURCE = dict(url='https://raw.githubusercontent.com/gregkh/linux/v6.18.56/drivers/i2c/busses/i2c-rk3x.c',
              bytes=37582, sha256='cee9b45dfee3fb4a7168585603c88df5e3422493425ac1bd35046d509d910423', license='GPL-2.0-only')
DEV = dict(url=base.BASE + 'linux-lts-dev-6.18.56-r0.apk', size=18028271,
           sha256='a2fd3f3a675e5e1c68b4dbea544fa7fe3e1ebbc0403bf1f8de6130ed6cdb1b80',
           version='6.18.56-r0', source_revision='b599422f9f640e35932a995ceedf52ec76b66963', license='GPL-2.0-only')
HEADER_BOUNDS = dict(count_limit=23000, individual_limit=4000000, total_limit=72000000,
                     expanded_limit=110000000, physical_count_limit=46000, metadata_limit=512, metadata_total_limit=2000000)
HEADER_STATS = dict(members=22261, total_member_bytes=70605705, expanded_tar_bytes=108446208,
                    physical_records=44522, physical_payload_bytes=70605705, extension_records=22261,
                    extension_metadata_bytes=1749396, largest_extension_metadata_bytes=213)
PINNED_HEADERS = {
    '.config': (269494, '4f33c7f3043a3d3e49794bd31aa4f7dce39951f2919e64ea11dfc1fbed0bcd17'),
    'Module.symvers': (1382573, '1db6b87e8f410f2b865e5fb37700c614d13f96852edd7dcfb8f952a2b96c03cb'),
    'include/generated/autoconf.h': (161382, '7ed06fb03128668300de92766488fde92e890d7bc73ef514527cf258d3dc323d'),
}
STOCK_MEMBER = 'lib/modules/' + RELEASE + '/kernel/drivers/i2c/i2c-dev.ko.gz'
STOCK_GZIP = (8580, 'd8287b5b3234089627e4c250b00df77da366b2405767187b01b0d6f54057918d')
STOCK_MODULE = (21719, '6381d4a9b257a836ce7f14a80ad1386ae5471e82a0b23715c848ba5362c159aa')
STOCK_CC = 'CONFIG_CC_VERSION_TEXT="cc (Alpine 15.2.0) 15.2.0"'
DEV_CC = 'CONFIG_CC_VERSION_TEXT="gcc (Alpine 15.2.0) 15.2.0"'
HELPERS = {
    'scripts/basic/fixdep': ['scripts/basic/fixdep.c'],
    'scripts/genksyms/genksyms': ['scripts/genksyms/genksyms.c', 'scripts/genksyms/parse.tab.c', 'scripts/genksyms/lex.lex.c'],
    'scripts/mod/modpost': ['scripts/mod/modpost.c', 'scripts/mod/file2alias.c', 'scripts/mod/sumversion.c', 'scripts/mod/symsearch.c'],
    'scripts/recordmcount': ['scripts/recordmcount.c'],
}
BUILD_ENV = dict(KBUILD_BUILD_TIMESTAMP='1970-01-01 00:00:00 +0000', KBUILD_BUILD_USER='labwired',
                 KBUILD_BUILD_HOST='rv1103', KBUILD_BUILD_VERSION='1', SOURCE_DATE_EPOCH='0', LC_ALL='C', TZ='UTC')
SIGNATURE = b'~Module signature appended~\n'
TOOL_VERSION_TIMEOUT = 10
TOOL_VERSION_LIMIT = 65536


def identity(data):
    return dict(bytes=len(data), sha256=base.digest(data))


def require_identity(data, expected, label):
    if (len(data), base.digest(data)) != expected:
        raise ValueError(f'{label} size/hash mismatch')


def cached_source(cache, offline=False, source_path=None):
    if source_path is not None:
        return base.checked_bytes(Path(source_path), SOURCE['bytes'], SOURCE['sha256'])
    cache = Path(cache)
    if cache.is_symlink() or any(p.is_symlink() for p in cache.parents): raise ValueError('symlink source cache')
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / (SOURCE['sha256'] + '.c')
    if path.exists() or path.is_symlink(): return base.checked_bytes(path, SOURCE['bytes'], SOURCE['sha256'])
    if offline: raise ValueError(f'missing offline source cache: {path}')
    with urllib.request.urlopen(SOURCE['url'], timeout=60) as response: data = response.read(SOURCE['bytes'] + 1)
    require_identity(data, (SOURCE['bytes'], SOURCE['sha256']), 'upstream source')
    fd, name = tempfile.mkstemp(prefix='.source-', dir=cache)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data); stream.flush(); os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name): os.unlink(name)
    return base.checked_bytes(path, SOURCE['bytes'], SOURCE['sha256'])


def archive_path(name):
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or '\0' in name or '\\' in name or str(path) == '.':
        raise ValueError('unsafe archive path')
    return str(path)


def link_target(name, target, entries):
    if not target or PurePosixPath(target).is_absolute() or '\0' in target or '\\' in target:
        raise ValueError('unsafe selected header link')
    parts = list(PurePosixPath(name).parent.parts)
    for part in PurePosixPath(target).parts:
        if part == '..':
            if not parts: raise ValueError('escaping selected header link')
            parts.pop()
        elif part != '.':
            parts.append(part)
            # POSIX resolves this component before a following '..'. Reject a
            # selected symlink now: collapsing x/.. first can hide an escape
            # when x points to a different directory (including the root).
            component = '/'.join(parts)
            if component in entries and entries[component][0] == 'link':
                raise ValueError('selected header link chains through another link')
    return '/'.join(parts)


def read_headers(data, **limits):
    """Validate the complete archive before materializing the whitelisted tree."""
    bounds = dict(HEADER_BOUNDS); bounds.update(limits)
    physical_keys = ('expanded_limit', 'metadata_limit', 'metadata_total_limit')
    expanded, stats = base.bounded_tar(data, bounds['physical_count_limit'], bounds['individual_limit'], bounds['total_limit'],
                                        *(bounds[key] for key in physical_keys))
    selected = {}; seen = set(); pkg = None; count = total = 0
    try:
        with tarfile.open(fileobj=io.BytesIO(expanded), mode='r:', ignore_zeros=True) as tar:
            for member in tar:
                name = archive_path(member.name)
                if name in seen: raise ValueError('duplicate archive path')
                seen.add(name); count += 1; total += member.size
                if count > bounds['count_limit'] or member.size < 0 or member.size > bounds['individual_limit'] or total > bounds['total_limit']:
                    raise ValueError('semantic archive bounds exceeded')
                is_header = name == HEADER_ROOT or name.startswith(HEADER_ROOT + '/')
                if not is_header and name != '.PKGINFO': continue
                if member.isfile():
                    content = tar.extractfile(member).read(bounds['individual_limit'] + 1)
                    if len(content) != member.size: raise ValueError('truncated selected header file')
                    entry = ('file', member.mode, content)
                elif is_header and member.isdir(): entry = ('dir', member.mode, b'')
                elif is_header and member.issym(): entry = ('link', member.mode, member.linkname)
                else: raise ValueError('unsupported selected archive type; expected regular/header directory/symlink')
                if name == '.PKGINFO': pkg = content
                elif name == HEADER_ROOT:
                    if entry[0] != 'dir': raise ValueError('header root must be directory')
                else: selected[name[len(HEADER_ROOT) + 1:]] = entry
    except RecursionError as error:
        raise ValueError('TAR extension metadata nesting exceeds parser recursion limit') from error
    except (tarfile.TarError, UnicodeError) as error:
        raise ValueError('invalid semantic TAR archive') from error
    for name, (kind, _, content) in selected.items():
        for parent in PurePosixPath(name).parents:
            if str(parent) != '.' and str(parent) in selected and selected[str(parent)][0] != 'dir':
                raise ValueError('selected archive entry has nondirectory/symlink parent')
        if kind == 'link':
            link_target(name, content, selected)
    stats.update(members=count, total_member_bytes=total)
    return selected, pkg, stats


def write_headers(entries, path):
    base.check_output(path); path = Path(path); path.mkdir()
    for name, (kind, _, content) in sorted(entries.items(), key=lambda item: (len(PurePosixPath(item[0]).parts), item[0])):
        dest = path / name; dest.parent.mkdir(parents=True, exist_ok=True)
        if kind == 'dir': dest.mkdir(exist_ok=True)
        elif kind == 'file': dest.write_bytes(content); dest.chmod(0o644)
        elif kind == 'link': dest.symlink_to(content)
        else: raise ValueError('unsupported header entry')


def tree_identity(path):
    """Hash names, types, permissions, link text and every regular file's bytes."""
    result = {}
    for parent, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            item = Path(parent) / name; mode = item.lstat().st_mode
            key = item.relative_to(path).as_posix()
            if stat.S_ISLNK(mode): result[key] = dict(mode=stat.S_IMODE(mode), link=os.readlink(item))
            elif stat.S_ISDIR(mode): result[key] = dict(mode=stat.S_IMODE(mode), directory=True)
            elif stat.S_ISREG(mode): result[key] = dict(mode=stat.S_IMODE(mode), **identity(item.read_bytes()))
            else: raise ValueError('nonregular staged header')
    return result


def freeze_tree(path):
    for parent, dirs, files in os.walk(path, topdown=False, followlinks=False):
        for name in files:
            item = Path(parent) / name
            if not item.is_symlink(): item.chmod(0o444)
        for name in dirs:
            item = Path(parent) / name
            if not item.is_symlink(): item.chmod(0o555)
    path.chmod(0o555)


def remove_owned_stage(path):
    # tempfile owns this entire newly created tree. Restore directory write bits
    # only there so read-only pristine inputs can be removed on success/failure.
    for parent, dirs, _ in os.walk(path, followlinks=False):
        Path(parent).chmod(0o700)
        for name in dirs:
            item = Path(parent) / name
            if not item.is_symlink(): item.chmod(0o700)
    shutil.rmtree(path)


def require_host_sources(path):
    result = {}
    for source in sorted({source for sources in HELPERS.values() for source in sources}):
        item = path / source
        if item.is_symlink() or not item.is_file(): raise ValueError(f'missing matching host source: {source}')
        result[source] = identity(item.read_bytes())
    return result


def verify_config_pair(stock, headers):
    stock_lines = stock.decode('ascii').splitlines(keepends=True); dev_lines = headers.decode('ascii').splitlines(keepends=True)
    if len(stock_lines) != len(dev_lines): raise ValueError('config has unexplained line-count difference')
    differences = [(a.rstrip('\n'), b.rstrip('\n')) for a, b in zip(stock_lines, dev_lines) if a != b]
    if differences != [(STOCK_CC, DEV_CC)]: raise ValueError('config difference exceeds exact compiler-line allowlist')
    required = ('CONFIG_MODVERSIONS=y', 'CONFIG_BASIC_MODVERSIONS=y', 'CONFIG_RANDSTRUCT_NONE=y',
                'CONFIG_GCC_PLUGINS=y', '# CONFIG_MODULE_SIG_FORCE is not set')
    for option in required:
        if stock.decode('ascii').splitlines().count(option) != 1: raise ValueError(f'original module policy missing: {option}')
    return dict(stock=STOCK_CC, headers=DEV_CC, stock_identity=identity(stock), headers_identity=identity(headers),
                allowed_difference='only the exact observed CONFIG_CC_VERSION_TEXT line; neither original config edited')


def elf_sections(data):
    if len(data) < 52 or data[:7] != b'\x7fELF\x01\x01\x01': raise ValueError('module must be ELF32 little-endian')
    values = struct.unpack_from('<16sHHIIIIIHHHHHH', data)
    _, kind, machine, version, entry, phoff, shoff, flags, ehsize, phsize, phnum, shsize, shnum, shstr = values
    if (kind, machine, version, entry, ehsize, phoff, phnum) != (1, 40, 1, 0, 52, 0, 0): raise ValueError('ELF must be ARM ET_REL with no program headers')
    if flags != 0x05000000: raise ValueError('module ELF EABI must be version5 with pinned flags')
    if shsize != 40 or not 1 < shnum <= 256 or not 0 < shstr < shnum or shoff < 52 or shoff + shsize * shnum > len(data):
        raise ValueError('ELF section table range')
    sections = [struct.unpack_from('<10I', data, shoff + i * 40) for i in range(shnum)]
    for section in sections:
        if section[1] != 8 and (section[4] > len(data) or section[5] > len(data) - section[4]): raise ValueError('ELF section payload range')
    strings_header = sections[shstr]
    if strings_header[1] != 3: raise ValueError('ELF section name table type')
    strings = data[strings_header[4]:strings_header[4] + strings_header[5]]
    result = {}
    for section in sections[1:]:
        start = section[0]; end = strings.find(b'\0', start)
        if start >= len(strings) or end < 0: raise ValueError('ELF section name range')
        try: name = strings[start:end].decode('ascii')
        except UnicodeError as error: raise ValueError('ELF section name encoding') from error
        if not name or name in result: raise ValueError('ELF duplicate/empty section name')
        result[name] = (section[1], data[section[4]:section[4] + section[5]] if section[1] != 8 else b'')
    return result


def symvers_map(data):
    result = {}
    for line in data.decode('ascii').splitlines():
        fields = line.split()
        if len(fields) not in (4, 5) or not re.fullmatch(r'0x[0-9a-fA-F]{8}', fields[0]): raise ValueError('invalid Module.symvers record')
        symbol = fields[1]
        if symbol in result: raise ValueError('duplicate Module.symvers symbol')
        result[symbol] = int(fields[0], 16)
    return result


def verify_module(data, symvers, expected_name, expected_import_count=None):
    if len(data) > 4000000: raise ValueError('module ELF byte bound')
    sections = elf_sections(data)
    if '.modinfo' not in sections or sections['.modinfo'][0] != 1: raise ValueError('ELF missing .modinfo')
    information = {}
    for record in sections['.modinfo'][1].split(b'\0'):
        if not record: continue
        try: key, value = record.decode('ascii').split('=', 1)
        except (UnicodeError, ValueError) as error: raise ValueError('invalid ELF modinfo') from error
        if key in ('name', 'depends', 'vermagic', 'license') and key in information: raise ValueError('duplicate module identity')
        information[key] = value
    if information.get('vermagic') != VERMAGIC: raise ValueError('module vermagic mismatch (including trailing space)')
    if information.get('name') != expected_name: raise ValueError('module name mismatch')
    if information.get('depends') != '': raise ValueError('unexpected module dependency')
    if information.get('license') != {'i2c_dev': 'GPL', 'i2c_rk3x': 'GPL v2'}.get(expected_name): raise ValueError('module license mismatch')
    if '__versions' not in sections or sections['__versions'][0] != 1: raise ValueError('missing ELF import versions')
    versions = sections['__versions'][1]
    if not versions or len(versions) % 64: raise ValueError('invalid ELF import version records')
    known = symvers_map(symvers); imports = {}
    for offset in range(0, len(versions), 64):
        crc = struct.unpack_from('<I', versions, offset)[0]; raw = versions[offset + 4:offset + 64]
        end = raw.find(b'\0')
        if end <= 0 or any(raw[end:]): raise ValueError('invalid ELF import name padding')
        try: name = raw[:end].decode('ascii')
        except UnicodeError as error: raise ValueError('invalid ELF import encoding') from error
        if name in imports: raise ValueError('duplicate ELF import symbol')
        if known.get(name) != crc: raise ValueError(f'ELF import CRC mismatch: {name}')
        imports[name] = f'0x{crc:08x}'
    if expected_import_count is not None and len(imports) != expected_import_count: raise ValueError('ELF import count mismatch')
    return dict(**identity(data), elf='ELF32 little-endian ARM ET_REL EABI5', vermagic=information['vermagic'],
                name=expected_name, license=information['license'], dependencies=[], imports=dict(sorted(imports.items())), import_count=len(imports),
                signature_present=data.endswith(SIGNATURE), crc_validation='actual ELF __versions against untouched matching Module.symvers')


def inflate_module(data, limit=STOCK_MODULE[0]):
    try:
        with gzip.GzipFile(fileobj=io.BytesIO(data)) as stream: expanded = stream.read(limit + 1)
    except (OSError, EOFError) as error: raise ValueError('invalid stock module gzip') from error
    if len(expanded) > limit: raise ValueError('stock module expanded byte bound')
    return expanded


def module_cpio(modules):
    if set(modules) != {'i2c-dev.ko', 'i2c-rk3x.ko'}: raise ValueError('overlay must contain exactly the two matching modules')
    directory = 'lib/modules/' + RELEASE
    entries = {name: (stat.S_IFDIR | 0o755, b'') for name in ('.', 'lib', 'lib/modules', directory)}
    entries.update({directory + '/' + name: (stat.S_IFREG | 0o644, content) for name, content in modules.items()})
    output = bytearray()
    for inode, (name, (mode, body)) in enumerate(sorted(entries.items()) + [('TRAILER!!!', (0, b''))], 1):
        encoded = name.encode() + b'\0'
        fields = [inode, mode, 0, 0, 2 if stat.S_ISDIR(mode) else 1, 0, len(body), 0, 0, 0, 0, len(encoded), 0]
        output.extend(b'070701' + ''.join(f'{field:08x}' for field in fields).encode() + encoded)
        output.extend(bytes(-len(output) % 4)); output.extend(body); output.extend(bytes(-len(output) % 4))
    return bytes(output)


def disk_guard(path, minimum=600 * 1024 * 1024):
    if shutil.disk_usage(path).free < minimum: raise ValueError('insufficient disk for bounded disposable module build')


def run_command(command, cwd, environment, log, timeout=180, log_limit=8 * 1024 * 1024):
    # A pipe bounds allocation/output before disk writes; the entire owned
    # process group is stopped on timeout/failure, including make's children.
    log.write((json.dumps(command) + '\n').encode()); log.flush()
    process = None; selector = selectors.DefaultSelector()
    try:
        process = subprocess.Popen(command, cwd=cwd, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   start_new_session=True, bufsize=0)
        selector.register(process.stdout, selectors.EVENT_READ)
        deadline = time.monotonic() + timeout
        while selector.get_map():
            # A wrapper may exit while children retain the stdout pipe. Its
            # exit finishes the owned command, so stop those children now and
            # then drain the bounded bytes already in the pipe.
            if process.poll() is not None:
                try: os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError: pass
            remaining = deadline - time.monotonic()
            if remaining <= 0: raise ValueError('build timeout')
            for key, _ in selector.select(min(remaining, 0.1)):
                chunk = os.read(key.fd, 65536)
                if not chunk:
                    selector.unregister(key.fileobj); continue
                allowance = log_limit - log.tell()
                log.write(chunk[:max(0, allowance)]); log.flush()
                if len(chunk) > allowance: raise ValueError('build log byte bound exceeded')
        remaining = deadline - time.monotonic()
        if remaining <= 0: raise ValueError('build timeout')
        if process.wait(timeout=remaining) != 0: raise ValueError('build exit status ' + str(process.returncode))
    except (ValueError, subprocess.SubprocessError, OSError) as error:
        log.flush()
        with Path(log.name).open('rb') as saved:
            saved.seek(max(0, saved.seek(0, 2) - 16384)); tail = saved.read(16384).decode('utf-8', errors='replace')
        raise ValueError(f'bounded build command failed: {command[0]}: {error}\n{tail}') from error
    finally:
        selector.close()
        if process is not None:
            try: os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            process.wait(timeout=5)
            process.stdout.close()


def tool_identity(name):
    resolved = shutil.which(name)
    if resolved is None: raise ValueError(f'missing build tool: {name}')
    path = Path(resolved).resolve()
    command = [str(path), '--version']
    environment = {key: os.environ[key] for key in ('PATH', 'TMPDIR') if key in os.environ}
    environment.update(LC_ALL='C', TZ='UTC')
    header_bytes = len((json.dumps(command) + '\n').encode())
    try:
        with tempfile.NamedTemporaryFile(mode='w+b', prefix='.rv1103-tool-version-') as log:
            run_command(command, path.parent, environment, log, timeout=TOOL_VERSION_TIMEOUT,
                        log_limit=header_bytes + TOOL_VERSION_LIMIT)
            log.seek(header_bytes); output = log.read(TOOL_VERSION_LIMIT + 1)
        if len(output) > TOOL_VERSION_LIMIT: raise ValueError('tool version byte bound exceeded')
        version = output.decode('utf-8')
    except (ValueError, OSError, UnicodeError) as error:
        raise ValueError(f'cannot identify tool: {name}: {error}') from error
    return dict(path=str(path), version=version, **identity(path.read_bytes()), version_command=command,
                version_output=identity(output), version_capture='combined stdout/stderr',
                version_timeout_seconds=TOOL_VERSION_TIMEOUT, version_byte_limit=TOOL_VERSION_LIMIT,
                version_environment=environment)


def resolve_cross_prefix(prefix):
    # Keep the selected gcc/as/ld aliases together. Resolving gcc's symlink may
    # produce a versioned basename that is not the toolchain's shared prefix.
    gcc = shutil.which(prefix + 'gcc')
    if gcc is None: raise ValueError('missing selected cross compiler')
    return str(Path(gcc).absolute()).removesuffix('gcc')


def host_elf(data):
    expected = {'x86_64': (2, 62), 'aarch64': (2, 183)}.get(platform.machine())
    if expected is None: raise ValueError('unsupported native helper host architecture')
    if len(data) < 20 or data[:4] != b'\x7fELF' or data[5:7] != b'\x01\x01' or (data[4], struct.unpack_from('<H', data, 18)[0]) != expected:
        raise ValueError('rebuilt helper is not native host ELF; extracted ARM utilities cannot run')


def build_module(pristine, entries, source, work, tools):
    disk_guard(work.parent)
    work.mkdir(); headers = work / 'headers'; module = work / 'module'
    shutil.copytree(pristine, headers, symlinks=True)
    for parent, dirs, files in os.walk(headers, followlinks=False):
        Path(parent).chmod(0o755)
        for name in files:
            item = Path(parent) / name
            if item.is_symlink(): continue
            original = entries[item.relative_to(headers).as_posix()]
            # APK ELF files are never executable here. Only original text scripts
            # retain execute permission; four matching helpers are rebuilt below.
            executable = original[1] & 0o111 and not item.read_bytes().startswith(b'\x7fELF')
            item.chmod(0o755 if executable else 0o644)
    protected_names = [name for name, entry in entries.items() if entry[0] == 'file' and
                       (name in ('.config', 'Module.symvers') or name.startswith(('include/generated/', 'arch/arm/include/generated/')))]
    protected = {name: identity((headers / name).read_bytes()) for name in protected_names}
    sources = require_host_sources(headers)
    environment = {key: os.environ[key] for key in ('PATH', 'TMPDIR') if key in os.environ}
    environment.update(BUILD_ENV)
    maps = [f'-fdebug-prefix-map={work}=/rv1103-build', f'-ffile-prefix-map={work}=/rv1103-build', f'-fmacro-prefix-map={work}=/rv1103-build']
    commands = []; helpers = {}; log_path = work / 'build.log'
    with log_path.open('wb') as log:
        for target, inputs in HELPERS.items():
            command = [tools['hostcc']['path'], '-O2', *maps,
                       *['-I' + str(headers / name) for name in ('scripts/include', 'scripts/genksyms', 'scripts/mod')],
                       '-o', str(headers / target), *[str(headers / name) for name in inputs]]
            commands.append(command); run_command(command, work, environment, log)
            binary = (headers / target).read_bytes(); host_elf(binary)
            helpers[target] = dict(**identity(binary), sources={name: sources[name] for name in inputs}, elf='native host', host_architecture=platform.machine())
        module.mkdir(); (module / 'i2c-rk3x.c').write_bytes(source); (module / 'Makefile').write_bytes(b'obj-m += i2c-rk3x.o\n')
        command = [tools['make']['path'], '-j1', '-C', str(headers), 'M=' + str(module), 'ARCH=arm',
                   'CROSS_COMPILE=' + tools['cross_prefix'], 'HOSTCC=' + tools['hostcc']['path'],
                   'CONFIG_GCC_PLUGINS=', 'KCFLAGS=' + ' '.join(maps), 'HOSTCFLAGS=-O2 ' + ' '.join(maps), 'modules']
        commands.append(command); run_command(command, work, environment, log)
    if {name: identity((headers / name).read_bytes()) for name in protected_names} != protected:
        raise ValueError('build modified matching config/symvers/generated headers')
    require_identity((module / 'i2c-rk3x.c').read_bytes(), (SOURCE['bytes'], SOURCE['sha256']), 'build source')
    ko = (module / 'i2c-rk3x.ko').read_bytes()
    return ko, dict(directory=str(work), commands=commands, environment=environment, helpers=helpers,
                    protected_headers_unchanged=True, protected_headers=protected), log_path.read_bytes()


def publish(output, files, receipt):
    if receipt.get('guest_status') != 'NOT_RUN': raise ValueError('preparation cannot claim guest execution')
    if receipt.get('preparation') != 'VERIFIED': raise ValueError('preparation guards must finish before publication')
    base.check_output(output); output = Path(output); output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.rv1103-modules-output-', dir=output.parent) as tmp:
        stage = Path(tmp) / 'artifacts'; stage.mkdir()
        for name, content in files.items():
            if archive_path(name) != name: raise ValueError('noncanonical output artifact name')
            dest = stage / name; dest.parent.mkdir(parents=True, exist_ok=True); dest.write_bytes(content)
        (stage / 'module-manifest.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
        base.check_output(output); os.rename(stage, output)


def prepare(output, cache, offline=False, source_path=None, hostcc='cc', cross_compile='arm-none-eabi-'):
    base.check_output(output)
    cache = Path(cache)
    if cache.is_symlink() or any(parent.is_symlink() for parent in cache.parents): raise ValueError('symlink APK cache')
    dev_apk = base.cached_apk(cache, DEV['url'], DEV['sha256'], DEV['size'], offline)
    entries, pkginfo, dev_stats = read_headers(dev_apk)
    if any(dev_stats.get(key) != value for key, value in HEADER_STATS.items()): raise ValueError('pinned header archive statistics mismatch')
    if pkginfo is None: raise ValueError('missing selected .PKGINFO')
    base.verify_identity(pkginfo, 'linux-lts-dev', DEV['version'])
    if sum(entry[0] == 'link' for entry in entries.values()) != 19: raise ValueError('pinned header symlink count mismatch')
    for name, expected in PINNED_HEADERS.items():
        if name not in entries or entries[name][0] != 'file': raise ValueError('missing matching generated header/config/symvers')
        require_identity(entries[name][2], expected, name)
    source = cached_source(cache, offline, source_path)
    kernel = base.PACKAGES['linux-lts']
    kernel_apk = base.cached_apk(cache, base.BASE + 'linux-lts-' + kernel['version'] + '.apk', kernel['sha256'], kernel['size'], offline)
    names = {'.PKGINFO': kernel['members']['.PKGINFO'], 'boot/config-' + RELEASE: kernel['members']['boot/config-' + RELEASE], STOCK_MEMBER: STOCK_GZIP}
    stock_stats = {}; members = base.selected_members(kernel_apk, names, kernel['count'], kernel['individual'], kernel['total'], statistics=stock_stats, **kernel['tar_bounds'])
    if (stock_stats['members'], stock_stats['total_member_bytes']) != (3717, 80605004): raise ValueError('pinned stock archive statistics mismatch')
    base.verify_identity(members['.PKGINFO'], 'linux-lts', kernel['version'])
    for name, (size, sha) in names.items():
        if len(members[name]) != size or (sha and base.digest(members[name]) != sha): raise ValueError('stock selected member size/hash mismatch')
    stock = inflate_module(members[STOCK_MEMBER]); require_identity(stock, STOCK_MODULE, 'stock i2c-dev')
    config = verify_config_pair(members['boot/config-' + RELEASE], entries['.config'][2])
    symvers = entries['Module.symvers'][2]
    stock_validation = verify_module(stock, symvers, 'i2c_dev')
    if not stock_validation['signature_present']: raise ValueError('pinned stock module signature missing')
    tools = dict(hostcc=tool_identity(hostcc), gcc=tool_identity(cross_compile + 'gcc'),
                 assembler=tool_identity(cross_compile + 'as'), linker=tool_identity(cross_compile + 'ld'), make=tool_identity('make'))
    tools['cross_prefix'] = resolve_cross_prefix(cross_compile)
    output = Path(output); output.parent.mkdir(parents=True, exist_ok=True); disk_guard(output.parent)
    stage = Path(tempfile.mkdtemp(prefix='.rv1103-module-build-', dir=output.parent))
    try:
        pristine = stage / 'pristine'; write_headers(entries, pristine); require_host_sources(pristine); freeze_tree(pristine)
        before = tree_identity(pristine)
        first, build_a, log_a = build_module(pristine, entries, source, stage / 'build-a', tools)
        first_validation = verify_module(first, symvers, 'i2c_rk3x', expected_import_count=48)
        if first_validation['signature_present']: raise ValueError('unexpected external module signature')
        second, build_b, log_b = build_module(pristine, entries, source, stage / 'build-b', tools)
        second_validation = verify_module(second, symvers, 'i2c_rk3x', expected_import_count=48)
        if first != second: raise ValueError('module bytes differ between independent build directories; inspect prefix maps/debug paths')
        if {name: value['sha256'] for name, value in build_a['helpers'].items()} != {name: value['sha256'] for name, value in build_b['helpers'].items()}:
            raise ValueError('native helper bytes differ between independent build directories')
        if before != tree_identity(pristine): raise ValueError('immutable original header tree changed')
        cpio = module_cpio({'i2c-dev.ko': stock, 'i2c-rk3x.ko': first})
        files = {'i2c-dev.ko': stock, 'i2c-rk3x.ko': first, 'modules.cpio': cpio, 'build-a.log': log_a, 'build-b.log': log_b}
        receipt = dict(schema=1, profile='rv1103-linux-optional-matching-modules', preparation='VERIFIED', guest_status='NOT_RUN',
                       physical_acceptance='UNAVAILABLE', kernel_release=RELEASE, packages={'linux-lts-dev': DEV, 'linux-lts': kernel},
                       preparer={'script': identity(Path(__file__).read_bytes()), 'base_script': identity(Path(base.__file__).read_bytes())},
                       upstream_source=SOURCE, selected_stock_compressed=identity(members[STOCK_MEMBER]),
                       archive_statistics={'linux-lts-dev': dev_stats, 'linux-lts': stock_stats}, config=config, tools=tools,
                       original_header_tree=dict(entries=len(before), sha256=base.digest(json.dumps(before, sort_keys=True).encode()), unchanged=True),
                       modules={'i2c-dev.ko': stock_validation, 'i2c-rk3x.ko': first_validation}, builds=[build_a, build_b],
                       reproducibility=dict(independent_builds=2, module_bytes_equal=True, native_helper_bytes_equal=True,
                                            second_module=second_validation, cpio_metadata='sorted newc, fixed inode/uid/gid/mode/nlink/mtime/devices; module files only'),
                       artifacts={name: identity(content) for name, content in files.items()},
                       build_overrides={'CONFIG_GCC_PLUGINS': '', 'ARCH': 'arm', 'jobs': 1, 'environment': BUILD_ENV},
                       guest_load_policy=dict(required='ordinary strict insmod for both modules; no force/modversion/CRC/signature bypass',
                                              stock_signed=True, external_signed=False, original_module_sig_force=False,
                                              taint='unsigned/out-of-tree external module may taint kernel; actual guest taint NOT_RUN'),
                       limitations=['host preparation/ELF ABI checks only; guest insertion and I2C transactions NOT_RUN',
                                    'cross compiler differs from stock Alpine GCC15.2 unless explicitly supplied matching toolchain; actual version recorded',
                                    'CONFIG_GCC_PLUGINS= build override omits original GCC plugin hardening; original config unchanged',
                                    'GCC13 lacks GCC15 counted_by support; vermagic/CRC equality does not prove equivalent compiler hardening',
                                    'optional modules-only cpio; base pinned fixture and loader unchanged; later deliberate basket profile required'])
        publish(output, files, receipt)
        return receipt
    finally:
        remove_owned_stage(stage)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--offline', action='store_true')
    parser.add_argument('--source', type=Path, help='existing pinned source (rehashes every use)')
    parser.add_argument('--hostcc', default='cc')
    parser.add_argument('--cross-compile', default='arm-none-eabi-')
    args = parser.parse_args()
    try:
        receipt = prepare(args.output, args.cache, args.offline, args.source, args.hostcc, args.cross_compile)
        print(json.dumps(dict(preparation=receipt['preparation'], output=str(args.output), guest_status=receipt['guest_status'])))
    except (ValueError, OSError, UnicodeError, RecursionError) as error:
        parser.exit(1, f'preparation failed: {error}\n')
