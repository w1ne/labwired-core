#!/usr/bin/env python3
"""Prepare pinned, reduced RV1103 Linux artifacts. No binaries enter the repository."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / 'examples/luckfox-pico-mini-a-linux'
BASE = 'https://dl-cdn.alpinelinux.org/alpine/v3.23/main/armv7/'
PACKAGES = {
    'linux-lts': dict(source_revision='b599422f9f640e35932a995ceedf52ec76b66963', license='GPL-2.0-only', version='6.18.56-r0', size=53474254, sha256='00059546e63df3dbadb80ff08f577dce1ef871b8cd800665d6404936a956eade', count=4000, individual=10000000, total=90000000, members={'.PKGINFO': (535, None), 'boot/config-6.18.56-0-lts': (269493, '4e64812402b287e13fd7513aa830432782a1ae94a5461bfe579999d1e18432fb'), 'boot/vmlinuz-lts': (8524288, 'eac9582162e912bdb577bba6ea224d98a29c997e52ee891383ea836aa704680a')}),
    'busybox-static': dict(source_revision='1e823a60eb85606954b3a5af5f8e5bbd1ea680cf', license='GPL-2.0-only', version='1.37.0-r30', size=628018, sha256='fb023d6a1e2dce9069b11b302e674c273b9e17d04d17b03ee16526475b0971e6', count=16, individual=1000000, total=1000000, members={'.PKGINFO': (568, None), 'bin/busybox.static': (886344, '1c0ed2db00c0bbdd796e4bdb4de58c564a3df8e61afc716652004b3ce0ee50f2')})
}
# The pinned APKs put a PAX header before each visible member. Permit twice the
# existing member ceiling, bound each short timestamp/path header to 512 bytes,
# and cap aggregate metadata separately from regular payload statistics.
PACKAGES['linux-lts']['tar_bounds'] = dict(expanded_limit=100000000, physical_count_limit=8000, metadata_limit=512, metadata_total_limit=512000)
PACKAGES['busybox-static']['tar_bounds'] = dict(expanded_limit=2000000, physical_count_limit=32, metadata_limit=512, metadata_total_limit=8192)
BOOTARGS = 'mem=64M cma=0 maxcpus=1 earlycon=uart8250,mmio32,0xff4c0000,115200n8 console=ttyS2,115200n8 rdinit=/init loglevel=8 panic=-1 clocksource.arm_arch_timer.evtstrm=0'

def digest(data): return hashlib.sha256(data).hexdigest()

def checked_bytes(path, size, sha):
    if path.is_symlink() or not path.is_file(): raise ValueError(f'nonregular artifact: {path}')
    with path.open('rb') as f: data = f.read(size + 1)
    if len(data) != size or digest(data) != sha: raise ValueError(f'size/hash mismatch: {path}')
    return data

def cached_apk(cache, url, sha, size, offline=False):
    cache = Path(cache)
    if cache.is_symlink(): raise ValueError('symlink cache')
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / (sha + '.apk')
    if path.exists() or path.is_symlink(): return checked_bytes(path, size, sha)
    if offline: raise ValueError(f'missing offline cache: {path}')
    # Bound both the transfer and allocation before extracting any member.
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read(size + 1)
    if len(data) != size or digest(data) != sha: raise ValueError('download size/hash mismatch')
    fd, tmp = tempfile.mkstemp(prefix='.apk-', dir=cache)
    try:
        with os.fdopen(fd, 'wb') as f: f.write(data); f.flush(); os.fsync(f.fileno())
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp): os.unlink(tmp)
    return checked_bytes(path, size, sha)

def bounded_tar(data, count_limit, individual_limit, total_limit, expanded_limit, metadata_limit, metadata_total_limit):
    # APKs concatenate gzip/TAR streams. Bound the entire expansion, including
    # padding and hidden metadata, before tarfile may interpret extensions.
    if min(count_limit, individual_limit, total_limit, expanded_limit, metadata_limit, metadata_total_limit) < 0:
        raise ValueError('negative archive bounds')
    try:
        with gzip.GzipFile(fileobj=io.BytesIO(data)) as compressed:
            expanded = compressed.read(expanded_limit + 1)
    except (OSError, EOFError) as error:
        raise ValueError('invalid gzip archive') from error
    if len(expanded) > expanded_limit: raise ValueError('expanded TAR byte bound exceeded')
    if len(expanded) % 512: raise ValueError('truncated physical TAR block')
    extensions = (tarfile.XHDTYPE, tarfile.XGLTYPE, tarfile.SOLARIS_XHDTYPE, tarfile.GNUTYPE_LONGNAME, tarfile.GNUTYPE_LONGLINK)
    offset = records = payload = metadata_records = metadata_bytes = largest_metadata = 0
    while offset < len(expanded):
        block = expanded[offset:offset + 512]; offset += 512
        if block == bytes(512): continue
        # frombuf parses exactly one physical header; it does not consume PAX,
        # GNU long-name/link bodies or recursively fetch a following header.
        try: member = tarfile.TarInfo.frombuf(block, 'utf-8', 'surrogateescape')
        except tarfile.HeaderError as error: raise ValueError('invalid physical TAR header') from error
        records += 1
        if records > count_limit: raise ValueError('physical TAR count bound exceeded')
        if member.size < 0: raise ValueError('negative physical TAR size')
        if member.type == tarfile.GNUTYPE_SPARSE:
            raise ValueError('unsupported GNU sparse physical records')
        if member.type in extensions:
            metadata_records += 1; metadata_bytes += member.size
            largest_metadata = max(largest_metadata, member.size)
            if member.size > metadata_limit or metadata_bytes > metadata_total_limit:
                raise ValueError('TAR extension metadata bound exceeded')
        else:
            payload += member.size
            if member.size > individual_limit or payload > total_limit:
                raise ValueError('physical TAR payload bounds exceeded')
        offset += (member.size + 511) // 512 * 512
        if offset > len(expanded): raise ValueError('truncated physical TAR payload')
    return expanded, dict(expanded_tar_bytes=len(expanded), physical_records=records, physical_payload_bytes=payload,
                          extension_records=metadata_records, extension_metadata_bytes=metadata_bytes, largest_extension_metadata_bytes=largest_metadata,
                          bounds=dict(expanded_tar_bytes=expanded_limit, physical_records=count_limit, individual_payload_bytes=individual_limit,
                                      total_payload_bytes=total_limit, individual_metadata_bytes=metadata_limit, total_metadata_bytes=metadata_total_limit))

def selected_members(data, names, count_limit, individual_limit, total_limit, statistics=None, *, expanded_limit=None, physical_count_limit=None, metadata_limit=65536, metadata_total_limit=262144):
    # One header plus at most 511 bytes padding per physical record; three
    # 10-KiB TAR stream trailers; separately budgeted extension metadata.
    if physical_count_limit is None: physical_count_limit = count_limit
    if expanded_limit is None: expanded_limit = total_limit + physical_count_limit * 1024 + 3 * 10240 + metadata_total_limit
    expanded, physical = bounded_tar(data, physical_count_limit, individual_limit, total_limit, expanded_limit, metadata_limit, metadata_total_limit)
    selected = {}; seen = set(); count = total = 0
    try:
        with tarfile.open(fileobj=io.BytesIO(expanded), mode='r:', ignore_zeros=True) as tar:
            for member in tar:
                count += 1; total += member.size
                p = PurePosixPath(member.name)
                if p.is_absolute() or '..' in p.parts or member.name in seen:
                    raise ValueError('unsafe or duplicate archive member')
                seen.add(member.name)
                if count > count_limit or member.size > individual_limit or total > total_limit:
                    raise ValueError('archive bounds exceeded')
                if member.name in names:
                    if not member.isfile(): raise ValueError('selected member is not regular')
                    f = tar.extractfile(member)
                    content = f.read(individual_limit + 1)
                    if len(content) != member.size: raise ValueError('truncated member')
                    selected[member.name] = content
    except RecursionError as error:
        raise ValueError('TAR extension metadata nesting exceeds parser recursion limit') from error
    if set(selected) != set(names): raise ValueError('missing selected members')
    if statistics is not None: statistics.update(members=count, total_member_bytes=total, **physical)
    return selected

def verify_identity(data, name, version):
    fields = {}
    for line in data.decode('utf-8').splitlines():
        if ' = ' in line:
            key, value = line.split(' = ', 1)
            if key in ('pkgname', 'pkgver', 'arch'):
                if key in fields: raise ValueError('duplicate package identity')
                fields[key] = value
    if fields != dict(pkgname=name, pkgver=version, arch='armv7'): raise ValueError('package identity mismatch')

def verify_config(data):
    text = data.decode('ascii'); lines = set(text.splitlines())
    for option in ('MMU', 'ARM_GIC', 'ARM_ARCH_TIMER', 'SERIAL_8250', 'SERIAL_8250_CONSOLE', 'SERIAL_8250_DW', 'BLK_DEV_INITRD'):
        if f'CONFIG_{option}=y' not in lines: raise ValueError(f'missing builtin {option}')
    if '# CONFIG_ARM_LPAE is not set' not in lines: raise ValueError('LPAE must be disabled')

def check_output(path):
    path = Path(path)
    if path.exists() or path.is_symlink(): raise ValueError('output must be a fresh nonsymlink directory')
    for parent in path.parents:
        if parent.is_symlink(): raise ValueError('symlink output parent')

def initramfs(busybox, init):
    # Deterministic newc: sorted fixed entries, fixed inode/uid/gid/time, no host metadata.
    entries = {'.': (stat.S_IFDIR | 0o755, b'', 0, 0)}
    for name in ('bin', 'proc', 'sys', 'tmp', 'dev'):
        entries[name] = (stat.S_IFDIR | (0o1777 if name == 'tmp' else 0o755), b'', 0, 0)
    entries['bin/busybox'] = (stat.S_IFREG | 0o755, busybox, 0, 0)
    entries['init'] = (stat.S_IFREG | 0o755, init, 0, 0)
    for applet in ('sh', 'mount', 'uname', 'cat', 'head', 'sleep', 'awk'):
        entries['bin/' + applet] = (stat.S_IFLNK | 0o777, b'busybox', 0, 0)
    entries['dev/console'] = (stat.S_IFCHR | 0o600, b'', 5, 1)
    entries['dev/null'] = (stat.S_IFCHR | 0o666, b'', 1, 3)
    output = bytearray()
    for inode, (name, (mode, body, rmajor, rminor)) in enumerate(sorted(entries.items()) + [('TRAILER!!!', (0, b'', 0, 0))], 1):
        encoded = name.encode() + b'\0'
        values = [inode, mode, 0, 0, 2 if stat.S_ISDIR(mode) else 1, 0, len(body), 0, 0, rmajor, rminor, len(encoded), 0]
        output.extend(b'070701' + ''.join(f'{v:08x}' for v in values).encode() + encoded)
        output.extend(b'\0' * (-len(output) % 4)); output.extend(body); output.extend(b'\0' * (-len(output) % 4))
    return bytes(output)

def validate_dtb(data, initrd_end):
    """Inspect actual compiler output, including references and final initrd end."""
    if len(data) < 40: raise ValueError('truncated DTB')
    header = struct.unpack_from('>10I', data)
    magic, size, off_s, off_n, off_r, version, compatible, cpu, len_n, len_s = header
    if (magic, size, version, cpu) != (0xd00dfeed, len(data), 17, 0) or compatible > 17:
        raise ValueError('DTB header')
    if not (40 <= off_r <= off_r + 16 <= off_s <= off_s + len_s <= off_n <= off_n + len_n == size):
        raise ValueError('DTB block ranges')
    if off_s % 4 or off_r % 8 or any(data[off_r:off_s]): raise ValueError('DTB reservations/alignment')
    names = data[off_n:off_n + len_n]; end_s = off_s + len_s
    def word(offset):
        if offset + 4 > end_s: raise ValueError('DTB word range')
        return struct.unpack_from('>I', data, offset)[0]
    def string(block, offset):
        if offset >= len(block): raise ValueError('DTB string offset')
        end = block.find(b'\0', offset)
        if end < 0: raise ValueError('DTB missing terminator')
        return block[offset:end].decode('ascii'), end + 1
    nodes = {}; stack = []; pos = off_s
    while pos < end_s:
        token = word(pos); pos += 4
        if token == 1:
            name, end = string(data[:end_s], pos); pos = (end + 3) & ~3
            if '/' in name or (not stack and (name or nodes)): raise ValueError('DTB node name')
            path = (stack[-1].rstrip('/') if stack else '') + '/' + name
            if path in nodes: raise ValueError('duplicate DTB node')
            nodes[path] = {}; stack.append(path)
        elif token == 2:
            if not stack: raise ValueError('DTB unbalanced node')
            stack.pop()
        elif token == 3:
            length, offset = word(pos), word(pos + 4); pos += 8
            name, ignored = string(names, offset)
            if not stack or pos + length > end_s or name in nodes[stack[-1]]: raise ValueError('DTB property range/duplicate')
            nodes[stack[-1]][name] = data[pos:pos+length]; pos = (pos + length + 3) & ~3
        elif token == 4: pass
        elif token == 9:
            if stack or pos != end_s: raise ValueError('DTB trailing structure')
            break
        else: raise ValueError('DTB token')
    else: raise ValueError('missing DTB end')
    def cells(*values): return struct.pack('>' + 'I' * len(values), *values)
    gic = '/interrupt-controller@ff1f1000'; uart = '/serial@ff4c0000'
    required = {
        '/': {'compatible': b'rockchip,rv1103\0', '#address-cells': cells(1), '#size-cells': cells(1)},
        '/aliases': {'serial2': (uart + '\0').encode()},
        '/chosen': {'stdout-path': b'serial2:115200n8\0', 'bootargs': (BOOTARGS + '\0').encode(), 'linux,initrd-start': cells(0x03100000), 'linux,initrd-end': cells(initrd_end)},
        '/memory@0': {'device_type': b'memory\0', 'reg': cells(0, 0x04000000)},
        '/cpus': {'#address-cells': cells(1), '#size-cells': cells(0)},
        '/cpus/cpu@0': {'device_type': b'cpu\0', 'compatible': b'arm,cortex-a7\0', 'reg': cells(0)},
        gic: {'compatible': b'arm,gic-400\0', '#interrupt-cells': cells(3), 'interrupt-controller': b'', 'reg': cells(0xff1f1000,0x1000,0xff1f2000,0x2000)},
        '/timer': {'compatible': b'arm,armv7-timer\0', 'interrupts': cells(1,13,0x104,1,14,0x104), 'clock-frequency': cells(24000000)},
        '/clock24m': {'compatible': b'fixed-clock\0', '#clock-cells': cells(0), 'clock-frequency': cells(24000000)},
        uart: {'compatible': b'snps,dw-apb-uart\0', 'reg': cells(0xff4c0000,0x100), 'interrupts': cells(0,27,4), 'reg-shift': cells(2), 'reg-io-width': cells(4), 'clock-frequency': cells(24000000), 'clock-names': b'baudclk\0apb_pclk\0', 'status': b'okay\0'}
    }
    if set(nodes) != set(required): raise ValueError('DTB unsupported nodes')
    extras = {'/': {'model', 'interrupt-parent'}, gic: {'#address-cells', 'phandle'}, '/clock24m': {'phandle'}, uart: {'clocks'}}
    for path, props in nodes.items():
        if set(props) - set(required[path]) - extras.get(path, set()): raise ValueError('DTB unsupported properties')

    for path, props in required.items():
        for key, value in props.items():
            if nodes[path].get(key) != value: raise ValueError(f'DTB geometry mismatch {path}:{key}')
    clock = nodes['/clock24m'].get('phandle'); interrupt = nodes[gic].get('phandle')
    if not clock or not interrupt or len(clock) != 4 or len(interrupt) != 4 or clock == interrupt or clock in (bytes(4), b'\xff' * 4) or interrupt in (bytes(4), b'\xff' * 4):
        raise ValueError('DTB invalid phandle')
    if nodes['/'].get('interrupt-parent') != interrupt or nodes[uart].get('clocks') != clock * 2:
        raise ValueError('DTB reference mismatch')

def prepare(output, cache, offline=False):
    check_output(output)
    artifacts = {}; archive_statistics = {}
    for name, package in PACKAGES.items():
        data = cached_apk(cache, BASE + name + '-' + package['version'] + '.apk', package['sha256'], package['size'], offline)
        stats = {}; selected = selected_members(data, package['members'], package['count'], package['individual'], package['total'], statistics=stats, **package['tar_bounds'])
        expected_stats = (3717, 80605004) if name == 'linux-lts' else (4, 887424)
        if (stats['members'], stats['total_member_bytes']) != expected_stats: raise ValueError('pinned archive statistics mismatch')
        archive_statistics[name] = stats
        verify_identity(selected['.PKGINFO'], name, package['version'])
        for member, (size, sha) in package['members'].items():
            content = selected[member]
            if len(content) != size or (sha and digest(content) != sha): raise ValueError('extracted size/hash mismatch')
        artifacts.update(selected)
    config = artifacts['boot/config-6.18.56-0-lts']; verify_config(config)
    kernel = artifacts['boot/vmlinuz-lts']
    if struct.unpack_from('<III', kernel, 0x24) != (0x016f2818, 0, len(kernel)): raise ValueError('zImage header mismatch')
    init = (EXAMPLE / 'init').read_bytes()
    cpio = initramfs(artifacts['bin/busybox.static'], init)
    end = 0x03100000 + len(cpio)
    if end > 0x04000000: raise ValueError('initramfs exceeds RAM')
    dts = (EXAMPLE / 'rv1103-direct.dts').read_text().replace('@INITRD_END@', f'0x{end:08x}')
    output = Path(output); output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.rv1103-', dir=output.parent) as tmp:
        stage = Path(tmp) / 'artifacts'; stage.mkdir()
        for name, content in [('zImage', kernel), ('kernel.config', config), ('initramfs.cpio', cpio), ('init', init), ('rv1103-direct.dts', dts.encode())]:
            (stage / name).write_bytes(content)
        subprocess.run(['dtc', '-I', 'dts', '-O', 'dtb', '-o', str(stage / 'rv1103-direct.dtb'), str(stage / 'rv1103-direct.dts')], check=True)
        validate_dtb((stage / 'rv1103-direct.dtb').read_bytes(), end)
        files = {}
        for name, address in [('zImage', 0x02000000), ('kernel.config', None), ('initramfs.cpio', 0x03100000), ('rv1103-direct.dtb', 0x03000000), ('init', None), ('rv1103-direct.dts', None)]:
            data = (stage / name).read_bytes(); files[name] = dict(bytes=len(data), sha256=digest(data), address=address)
        receipt = dict(schema=1, board='luckfox-pico-mini-a-rv1103', ram_base=0, ram_bytes=0x04000000, cpu_hz_default=1200000000, timer_hz=24000000, decompression_start=0x8000, decompression_end=0x016e5850, bootargs=BOOTARGS, packages=PACKAGES, archive_statistics=archive_statistics, artifacts=files, preparation='VERIFIED', guest_boot='NOT_RUN', physical_acceptance='UNAVAILABLE', reductions=['direct zImage boot; no boot ROM or SD/BSP', 'one CPU; no CRU/reset/pinctrl/DMA/FIQ debugger', 'instruction ticks are functional simulation, not hardware timing', 'no I2C, ESP, RF or smartbasket application acceptance'])
        (stage / 'artifact-manifest.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
        check_output(output); os.rename(stage, output)
    return receipt

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    try:
        receipt = prepare(args.output, args.cache, args.offline)
        print(json.dumps(dict(preparation=receipt['preparation'], output=str(args.output), guest_boot=receipt['guest_boot'])))
    except (ValueError, OSError, tarfile.TarError, subprocess.CalledProcessError) as e:
        parser.exit(1, f'preparation failed: {e}\n')
