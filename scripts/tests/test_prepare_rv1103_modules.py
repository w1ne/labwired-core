"""Negative guards for the optional matching-module preparation step."""
import gzip
import importlib.util
import io
import os
from pathlib import Path
import shutil
import signal
import stat
import struct
import sys
import tarfile
import tempfile
import time
import unittest
from unittest import mock

SCRIPT = Path(__file__).parents[1] / 'prepare-rv1103-modules.py'
HEADER = 'usr/src/linux-headers-6.18.56-0-lts'
VERMAGIC = '6.18.56-0-lts SMP mod_unload modversions ARMv7 p2v8 '
STOCK_CC = b'CONFIG_CC_VERSION_TEXT="cc (Alpine 15.2.0) 15.2.0"\n'
DEV_CC = b'CONFIG_CC_VERSION_TEXT="gcc (Alpine 15.2.0) 15.2.0"\n'
POLICY = b'CONFIG_MODVERSIONS=y\nCONFIG_BASIC_MODVERSIONS=y\nCONFIG_RANDSTRUCT_NONE=y\nCONFIG_GCC_PLUGINS=y\n# CONFIG_MODULE_SIG_FORCE is not set\n'


def apk(entries):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode='w:gz', format=tarfile.PAX_FORMAT) as tar:
        for name, kind, content in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            if kind == tarfile.REGTYPE:
                member.size = len(content)
                tar.addfile(member, io.BytesIO(content))
            else:
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE): member.linkname = content
                tar.addfile(member)
    return stream.getvalue()


def module_elf(versions=(('probe', 0x12345678),), vermagic=VERMAGIC, name='i2c_rk3x', depends='', machine=40, license='GPL v2'):
    """Construct a small actual ELF32/ARM section table, not generated .mod.c."""
    modinfo = f'license={license}\0name={name}\0depends={depends}\0vermagic={vermagic}\0'.encode()
    imports = b''.join(struct.pack('<I', crc) + symbol.encode().ljust(60, b'\0') for symbol, crc in versions)
    strings = b'\0.modinfo\0__versions\0.shstrtab\0'
    payload = bytearray(bytes(52))
    sections = [(0, 0, 0, 0, 0, 0, 0, 0, 0, 0)]
    for offset, data in [(1, modinfo), (10, imports), (21, strings)]:
        start = len(payload)
        payload.extend(data)
        sections.append((offset, 3 if data is strings else 1, 0, 0, start, len(data), 0, 0, 1, 0))
    table = len(payload)
    for section in sections: payload.extend(struct.pack('<10I', *section))
    ident = b'\x7fELF\x01\x01\x01' + bytes(9)
    payload[:52] = struct.pack('<16sHHIIIIIHHHHHH', ident, 1, machine, 1, 0, 0, table, 0x05000000, 52, 0, 0, 40, 4, 3)
    return bytes(payload)


def cpio_records(data):
    records = []; offset = 0
    while offset < len(data):
        if data[offset:offset + 6] != b'070701': raise AssertionError('newc magic')
        fields = [int(data[offset + 6 + i * 8:offset + 14 + i * 8], 16) for i in range(13)]
        offset += 110
        name = data[offset:offset + fields[11] - 1].decode()
        offset = (offset + fields[11] + 3) & ~3
        body = data[offset:offset + fields[6]]
        offset = (offset + fields[6] + 3) & ~3
        records.append((name, fields, body))
        if name == 'TRAILER!!!': break
    if offset != len(data): raise AssertionError('cpio trailing bytes')
    return records


class ModulePreparationTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), 'strict optional module preparer is missing')
        spec = importlib.util.spec_from_file_location('prepare_modules', SCRIPT)
        self.prep = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.prep)

    def headers(self, entries, **bounds):
        return self.prep.read_headers(apk(entries), **bounds)

    def owned_child_gone(self, proc):
        # One read avoids the exists/read TOCTOU while an owned child exits.
        try:
            with proc.open('r') as status:
                state = status.read(4097)
        except (FileNotFoundError, ProcessLookupError):
            return True
        self.assertLessEqual(len(state), 4096, 'owned child stat exceeded bound')
        return state.rsplit(')', 1)[1].split()[0] == 'Z'

    def test_only_header_tree_is_selected_and_unselected_absolute_link_is_ignored(self):
        entries, pkg, stats = self.headers([
            (HEADER + '/include/a.h', tarfile.REGTYPE, b'header'),
            ('.PKGINFO', tarfile.REGTYPE, b'identity'),
            ('lib/modules/6.18.56-0-lts/build', tarfile.SYMTYPE, '/' + HEADER),
            ('irrelevant', tarfile.REGTYPE, b'unknown'),
        ])
        self.assertEqual(set(entries), {'include/a.h'})
        self.assertEqual(pkg, b'identity')
        self.assertEqual(stats['members'], 4)

    def test_safe_relative_header_links_include_dangling_intratree_targets(self):
        entries, _, _ = self.headers([
            (HEADER + '/include/uapi/a.h', tarfile.REGTYPE, b'a'),
            (HEADER + '/include/linux/a.h', tarfile.SYMTYPE, '../uapi/a.h'),
            (HEADER + '/scripts/dtc/include-prefixes/arm', tarfile.SYMTYPE, '../../../arch/arm/boot/dts'),
        ])
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'headers'
            self.prep.write_headers(entries, output)
            self.assertEqual((output / 'include/linux/a.h').read_bytes(), b'a')
            self.assertTrue((output / 'scripts/dtc/include-prefixes/arm').is_symlink())
            self.assertFalse((output / 'scripts/dtc/include-prefixes/arm').exists())

    def test_unsafe_archive_names_are_rejected_even_when_unselected(self):
        for name in ('/outside', '../outside', 'a/../outside'):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'path'):
                self.headers([(name, tarfile.REGTYPE, b'a')])

    def test_canonical_duplicate_archive_paths_are_rejected(self):
        for names in [(HEADER + '/a', HEADER + '/a'), (HEADER + '/a', './' + HEADER + '/a')]:
            with self.subTest(names=names), self.assertRaisesRegex(ValueError, 'duplicate'):
                self.headers([(name, tarfile.REGTYPE, b'a') for name in names])

    def test_selected_absolute_and_escaping_links_are_rejected(self):
        for target in ('/outside', '../../../../../outside'):
            with self.subTest(target=target), self.assertRaisesRegex(ValueError, 'link'):
                self.headers([(HEADER + '/include/a', tarfile.SYMTYPE, target)])

    def test_selected_special_and_hardlink_members_are_rejected(self):
        for kind in (tarfile.LNKTYPE, tarfile.FIFOTYPE, tarfile.CHRTYPE, tarfile.BLKTYPE):
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, 'type|regular'):
                self.headers([(HEADER + '/include/a', kind, 'target')])

    def test_symlink_parent_is_rejected_in_both_archive_orders(self):
        records = [(HEADER + '/include', tarfile.SYMTYPE, 'arch'), (HEADER + '/include/a.h', tarfile.REGTYPE, b'a')]
        for entries in (records, records[::-1]):
            with self.subTest(entries=entries), self.assertRaisesRegex(ValueError, 'parent'):
                self.headers(entries)

    def test_links_cannot_chain_through_another_symlink(self):
        with self.assertRaisesRegex(ValueError, 'link'):
            self.headers([(HEADER + '/inside', tarfile.SYMTYPE, 'include'), (HEADER + '/a', tarfile.SYMTYPE, 'inside/a')])

    def test_symlink_then_dotdot_cannot_read_an_outside_sentinel(self):
        # POSIX resolves x -> '.' before '..', so x/../outside actually escapes
        # the extraction root. A lexical collapse to 'outside' is incorrect.
        for target in ('x/../outside', './x/../outside', 'x//../outside'):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as tmp:
                sentinel = Path(tmp) / 'outside'; sentinel.write_bytes(b'outside-header-sentinel')
                try:
                    entries, _, _ = self.headers([
                        (HEADER + '/x', tarfile.SYMTYPE, '.'),
                        (HEADER + '/a', tarfile.SYMTYPE, target),
                    ])
                except ValueError as error:
                    self.assertRegex(str(error), 'link'); continue
                output = Path(tmp) / 'headers'; self.prep.write_headers(entries, output)
                self.assertEqual((output / 'a').read_bytes(), sentinel.read_bytes(), 'fixture must reproduce actual escape')
                self.fail('selected symlink plus .. was accepted and read the outside sentinel')

    def test_symlink_component_is_checked_before_dotdot_even_for_intratree_result(self):
        # Chained selected links are forbidden even when both lexical and real
        # resolution would stay inside the tree.
        for name, target in [('a', 'x/../inside'), ('include/a', '../x/../inside')]:
            with self.subTest(name=name, target=target), self.assertRaisesRegex(ValueError, 'link'):
                self.headers([
                    (HEADER + '/include', tarfile.DIRTYPE, ''),
                    (HEADER + '/deep', tarfile.DIRTYPE, ''),
                    (HEADER + '/deep/inner', tarfile.DIRTYPE, ''),
                    (HEADER + '/x', tarfile.SYMTYPE, 'deep/inner'),
                    (HEADER + '/' + name, tarfile.SYMTYPE, target),
                ])

    def test_expansion_and_hidden_pax_bounds_precede_semantic_parse(self):
        data = apk([(HEADER + '/a', tarfile.REGTYPE, b'a')])
        with mock.patch.object(self.prep.tarfile, 'open', side_effect=AssertionError('semantic parser called')), self.assertRaisesRegex(ValueError, 'expanded'):
            self.prep.read_headers(data, expanded_limit=1024)
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w:gz', format=tarfile.PAX_FORMAT) as tar:
            m = tarfile.TarInfo(HEADER + '/a'); m.size = 1; m.pax_headers = {'comment': 'x' * 1024}
            tar.addfile(m, io.BytesIO(b'a'))
        with mock.patch.object(self.prep.tarfile, 'open', side_effect=AssertionError('semantic parser called')), self.assertRaisesRegex(ValueError, 'metadata'):
            self.prep.read_headers(stream.getvalue())

    def test_unknown_members_still_count_toward_semantic_and_payload_bounds(self):
        for bounds in ({'count_limit': 1}, {'total_limit': 1}, {'individual_limit': 1}):
            with self.subTest(bounds=bounds), self.assertRaisesRegex(ValueError, 'bound'):
                self.headers([('irrelevant', tarfile.REGTYPE, b'aa'), (HEADER + '/a', tarfile.REGTYPE, b'b')], **bounds)

    def test_recursive_pax_is_a_controlled_failure(self):
        data = apk([('pax', tarfile.REGTYPE, b'a')])
        with mock.patch.object(self.prep.tarfile, 'open', side_effect=RecursionError), self.assertRaisesRegex(ValueError, 'recursion|nesting'):
            self.prep.read_headers(data)

    def test_missing_host_sources_fail_before_any_compiler_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.object(self.prep.subprocess, 'run', side_effect=AssertionError('compiler executed')), self.assertRaisesRegex(ValueError, 'missing.*source'):
                self.prep.require_host_sources(Path(tmp))

    def test_source_hash_is_checked_and_missing_offline_cache_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'missing offline'):
                self.prep.cached_source(Path(tmp), offline=True)
            cached = Path(tmp) / (self.prep.SOURCE['sha256'] + '.c')
            for bad in (b'wrong', b'x' * self.prep.SOURCE['bytes']):
                cached.write_bytes(bad)
                with self.assertRaisesRegex(ValueError, 'size/hash'):
                    self.prep.cached_source(Path(tmp), offline=True)

    def test_only_exact_observed_compiler_config_line_difference_is_allowed(self):
        result = self.prep.verify_config_pair(STOCK_CC + POLICY, DEV_CC + POLICY)
        self.assertEqual(result['stock'], STOCK_CC.decode().strip())
        self.assertEqual(result['headers'], DEV_CC.decode().strip())
        for dev in (DEV_CC.replace(b'15.2.0', b'15.3.0') + POLICY, DEV_CC + POLICY + b'CONFIG_TEST=y\n', DEV_CC + POLICY.replace(b'CONFIG_GCC_PLUGINS=y', b'# CONFIG_GCC_PLUGINS is not set')):
            with self.subTest(dev=dev), self.assertRaisesRegex(ValueError, 'config'):
                self.prep.verify_config_pair(STOCK_CC + POLICY, dev)

    def test_modversions_and_original_signature_policy_are_required(self):
        for policy in (POLICY.replace(b'CONFIG_MODVERSIONS=y\n', b''), POLICY.replace(b'# CONFIG_MODULE_SIG_FORCE is not set', b'CONFIG_MODULE_SIG_FORCE=y'), POLICY.replace(b'CONFIG_RANDSTRUCT_NONE=y\n', b'')):
            with self.subTest(policy=policy), self.assertRaisesRegex(ValueError, 'policy'):
                self.prep.verify_config_pair(STOCK_CC + policy, DEV_CC + policy)

    def test_actual_elf_crc_and_vermagic_are_verified(self):
        result = self.prep.verify_module(module_elf(), b'0x12345678\tprobe\tvmlinux\tEXPORT_SYMBOL\n', 'i2c_rk3x')
        self.assertEqual(result['imports'], {'probe': '0x12345678'})
        self.assertEqual(result['vermagic'], VERMAGIC)
        self.assertFalse(result['signature_present'])

    def test_actual_pinned_rk3x_gpl_v2_and_stock_gpl_licenses(self):
        symvers = b'0x12345678 probe vmlinux EXPORT_SYMBOL\n'
        try: self.prep.verify_module(module_elf(license='GPL v2'), symvers, 'i2c_rk3x')
        except ValueError as error: self.fail(f'pinned unchanged RK3x source license was rejected: {error}')
        self.prep.verify_module(module_elf(name='i2c_dev', license='GPL'), symvers, 'i2c_dev')
        with self.assertRaisesRegex(ValueError, 'license'):
            self.prep.verify_module(module_elf(license='Proprietary'), symvers, 'i2c_rk3x')

    def test_owned_child_liveness_distinguishes_gone_zombie_live_and_other_errors(self):
        self.assertTrue(callable(getattr(self, 'owned_child_gone', None)), 'bounded owned-child liveness helper missing')
        self.assertFalse(self.owned_child_gone(Path(f'/proc/{os.getpid()}/stat')), 'live test process must remain live')
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(self.owned_child_gone(Path(tmp) / 'absent'))
            proc = Path(tmp) / 'stat'
            for state, expected in [('R', False), ('S', False), ('Z', True)]:
                proc.write_text(f'123 (name with spaces) {state} 1 0\n')
                self.assertEqual(self.owned_child_gone(proc), expected)
        for error in (FileNotFoundError(), ProcessLookupError()):
            with mock.patch.object(Path, 'open', side_effect=error):
                self.assertTrue(self.owned_child_gone(Path('/proc/owned/stat')))
        for error in (PermissionError(), OSError('other proc read error')):
            with mock.patch.object(Path, 'open', side_effect=error), self.assertRaises(type(error)):
                self.owned_child_gone(Path('/proc/owned/stat'))

    def test_timeout_terminates_owned_build_children(self):
        child = None
        try:
            with tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'log'
                command = [sys.executable, '-c', 'import subprocess,sys,time; p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"]); print(p.pid,flush=True); time.sleep(30)']
                with path.open('wb') as log, self.assertRaisesRegex(ValueError, 'failed|timeout'):
                    self.prep.run_command(command, Path(tmp), dict(os.environ), log, timeout=0.3)
                child = int(path.read_text().splitlines()[1])
                proc = Path(f'/proc/{child}/stat')
                gone = False
                for _ in range(20):
                    if self.owned_child_gone(proc):
                        gone = True
                        break
                    time.sleep(0.05)
                self.assertTrue(gone, 'timed-out build left a live owned child')
        finally:
            if child is not None:
                try: os.kill(child, signal.SIGKILL)
                except ProcessLookupError: pass

    def test_failed_build_preserves_bounded_diagnostic_tail(self):
        with tempfile.TemporaryDirectory() as tmp:
            with (Path(tmp) / 'log').open('wb') as log:
                with self.assertRaisesRegex(ValueError, 'actual compiler diagnostic'):
                    self.prep.run_command([sys.executable, '-c', 'print("actual compiler diagnostic",flush=True); raise SystemExit(2)'], Path(tmp), dict(os.environ), log)

    def test_build_output_is_bounded_before_writing_to_disk(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'log'
            command = [sys.executable, '-c', 'import os; os.write(1,b"x"*(9*1024*1024))']
            with path.open('wb') as log, self.assertRaisesRegex(ValueError, 'log byte bound'):
                self.prep.run_command(command, Path(tmp), dict(os.environ), log)
            self.assertEqual(path.stat().st_size, 8 * 1024 * 1024)

    def test_arm_elf_cannot_be_accepted_as_a_native_host_helper(self):
        with self.assertRaisesRegex(ValueError, 'native host ELF'):
            self.prep.host_elf(module_elf())

    def test_selected_cross_prefix_preserves_toolchain_aliases(self):
        # Versioned GCC symlink targets commonly have a different basename from
        # the prefix shared by gcc/as/ld. Kbuild must use the selected aliases.
        with tempfile.TemporaryDirectory() as tmp:
            prefix = str(Path(tmp) / 'selected-arm-')
            Path(prefix + 'gcc').symlink_to(shutil.which('cc'))
            Path(prefix + 'as').symlink_to(shutil.which('as'))
            Path(prefix + 'ld').symlink_to(shutil.which('ld'))
            resolved = self.prep.resolve_cross_prefix(prefix)
            self.assertTrue(Path(resolved + 'as').is_file(), 'selected gcc alias lost its matching assembler prefix')
            self.assertEqual(resolved, prefix)

    def test_tool_version_output_is_bounded(self):
        with tempfile.TemporaryDirectory() as tmp:
            tool = Path(tmp) / 'version-tool'
            tool.write_text('#!' + sys.executable + '\nimport os\nos.write(1,b"x"*(9*1024*1024))\n')
            tool.chmod(0o755)
            with self.assertRaisesRegex(ValueError, 'byte bound'):
                self.prep.tool_identity(str(tool))

    def test_tool_version_normal_exit_stops_owned_descendant(self):
        child = None
        try:
            with tempfile.TemporaryDirectory() as tmp:
                tool = Path(tmp) / 'version-tool'
                tool.write_text('#!' + sys.executable + '\nimport subprocess,sys\np=subprocess.Popen([sys.executable,"-c","import time;time.sleep(30)"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\nprint(p.pid,flush=True)\n')
                tool.chmod(0o755)
                result = self.prep.tool_identity(str(tool))
                child = int(result['version'].strip())
                proc = Path(f'/proc/{child}/stat')
                gone = False
                for _ in range(20):
                    if self.owned_child_gone(proc):
                        gone = True
                        break
                    time.sleep(0.05)
                self.assertTrue(gone, 'version identity left a live owned descendant')
        finally:
            if child is not None:
                try: os.kill(child, signal.SIGKILL)
                except ProcessLookupError: pass

    def test_tool_version_normal_exit_with_inherited_pipe_is_not_held_by_child(self):
        child = None; pid_file = None
        try:
            with tempfile.TemporaryDirectory() as tmp:
                tool = Path(tmp) / 'version-tool'
                pid_file = Path(tmp) / 'child.pid'
                tool.write_text('#!' + sys.executable + '\nimport subprocess,sys\np=subprocess.Popen([sys.executable,"-c","import time;time.sleep(30)"])\nopen(' + repr(str(pid_file)) + ',"w").write(str(p.pid))\nprint(p.pid,flush=True)\n')
                tool.chmod(0o755)
                started = time.monotonic()
                # The wrapper exits normally. Its child retaining stdout must
                # not keep identification open until the ten-second timeout.
                try: result = self.prep.tool_identity(str(tool))
                except ValueError as error:
                    child = int(pid_file.read_text())
                    self.fail(f'normal version-tool exit was held by owned child: {error}')
                child = int(result['version'].strip())
                self.assertLess(time.monotonic() - started, 3)
                proc = Path(f'/proc/{child}/stat')
                self.assertTrue(self.owned_child_gone(proc))
                if pid_file.exists(): pid_file.unlink()
        finally:
            if child is None and pid_file is not None and pid_file.exists(): child = int(pid_file.read_text())
            if child is not None:
                try: os.kill(child, signal.SIGKILL)
                except ProcessLookupError: pass

    def test_tool_version_timeout_stops_owned_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            tool = Path(tmp) / 'version-tool'; pid_file = Path(tmp) / 'child.pid'
            tool.write_text('#!' + sys.executable + '\nimport subprocess,sys,time\np=subprocess.Popen([sys.executable,"-c","import time;time.sleep(30)"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\nopen(' + repr(str(pid_file)) + ',"w").write(str(p.pid))\nprint("hanging version",flush=True)\ntime.sleep(30)\n')
            tool.chmod(0o755)
            try:
                with mock.patch.object(self.prep, 'TOOL_VERSION_TIMEOUT', 0.3, create=True), self.assertRaisesRegex(ValueError, 'identify|timeout'):
                    self.prep.tool_identity(str(tool))
                child = int(pid_file.read_text()); proc = Path(f'/proc/{child}/stat')
                gone = False
                for _ in range(20):
                    if self.owned_child_gone(proc):
                        gone = True
                        break
                    time.sleep(0.05)
                self.assertTrue(gone, 'timed-out version tool left a live owned child')
            finally:
                if pid_file.exists():
                    try: os.kill(int(pid_file.read_text()), signal.SIGKILL)
                    except ProcessLookupError: pass

    def test_wrong_vermagic_dependency_name_and_crc_fail(self):
        for data, symvers in [(module_elf(vermagic=VERMAGIC.rstrip()), b'0x12345678 probe vmlinux EXPORT_SYMBOL\n'), (module_elf(depends='fake'), b'0x12345678 probe vmlinux EXPORT_SYMBOL\n'), (module_elf(name='fake'), b'0x12345678 probe vmlinux EXPORT_SYMBOL\n'), (module_elf(), b'0x87654321 probe vmlinux EXPORT_SYMBOL\n'), (module_elf(), b'0x12345678 missing vmlinux EXPORT_SYMBOL\n')]:
            with self.subTest(data=data), self.assertRaisesRegex(ValueError, 'vermagic|depend|name|CRC'):
                self.prep.verify_module(data, symvers, 'i2c_rk3x')

    def test_malformed_elf_architecture_sections_and_duplicate_imports_fail(self):
        good = module_elf(); out_of_range = bytearray(good); struct.pack_into('<I', out_of_range, 32, len(good) + 10)
        wrong_eabi = bytearray(good); struct.pack_into('<I', wrong_eabi, 36, 0)
        for data in (b'notelf', module_elf(machine=62), bytes(out_of_range), bytes(wrong_eabi), module_elf(versions=(('probe', 1), ('probe', 1))), module_elf(versions=())):
            with self.subTest(data=data), self.assertRaisesRegex(ValueError, 'ELF|EABI|import|version'):
                self.prep.verify_module(data, b'0x00000001 probe vmlinux EXPORT_SYMBOL\n', 'i2c_rk3x')

    def test_duplicate_symvers_symbols_and_expected_import_count_fail(self):
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.prep.verify_module(module_elf(), b'0x12345678 probe vmlinux EXPORT_SYMBOL\n' * 2, 'i2c_rk3x')
        with self.assertRaisesRegex(ValueError, 'count'):
            self.prep.verify_module(module_elf(), b'0x12345678 probe vmlinux EXPORT_SYMBOL\n', 'i2c_rk3x', expected_import_count=48)

    def test_stock_module_gzip_expansion_is_bounded(self):
        with self.assertRaisesRegex(ValueError, 'expanded'):
            self.prep.inflate_module(gzip.compress(b'x' * 100), limit=99)
        self.assertEqual(self.prep.inflate_module(gzip.compress(b'ab'), limit=2), b'ab')

    def test_module_overlay_newc_has_only_fixed_sorted_module_entries(self):
        modules = {'i2c-rk3x.ko': b'rk', 'i2c-dev.ko': b'dev'}
        data = self.prep.module_cpio(modules)
        self.assertEqual(data, self.prep.module_cpio(dict(reversed(list(modules.items())))))
        records = cpio_records(data)
        names = [name for name, _, _ in records[:-1]]
        self.assertEqual(names, sorted(names))
        self.assertEqual({name: body for name, fields, body in records if stat.S_ISREG(fields[1])}, {'lib/modules/6.18.56-0-lts/i2c-dev.ko': b'dev', 'lib/modules/6.18.56-0-lts/i2c-rk3x.ko': b'rk'})
        for _, fields, _ in records:
            self.assertEqual((fields[2], fields[3], fields[5], fields[7], fields[8], fields[9], fields[10], fields[12]), (0,) * 8)
        with self.assertRaisesRegex(ValueError, 'module'):
            self.prep.module_cpio({'../evil': b'x', 'i2c-dev.ko': b'dev'})

    def test_pristine_tree_detects_mutation_and_extracted_elf_is_not_executable(self):
        entries, _, _ = self.headers([(HEADER + '/scripts/tool', tarfile.REGTYPE, b'\x7fELFarm'), (HEADER + '/include/a', tarfile.REGTYPE, b'a')])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'headers'; self.prep.write_headers(entries, path)
            before = self.prep.tree_identity(path)
            self.assertEqual((path / 'scripts/tool').stat().st_mode & 0o111, 0)
            (path / 'include/a').write_bytes(b'mutated')
            self.assertNotEqual(self.prep.tree_identity(path), before)

    def test_failed_prepare_is_atomic_and_existing_or_symlink_output_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'output'; cache = Path(tmp) / 'cache'
            with self.assertRaisesRegex(ValueError, 'missing offline'):
                self.prep.prepare(output, cache, offline=True)
            self.assertFalse(output.exists())
            output.mkdir()
            with self.assertRaisesRegex(ValueError, 'fresh'):
                self.prep.prepare(output, cache, offline=True)
            link = Path(tmp) / 'link'; link.symlink_to(Path(tmp), target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'symlink'):
                self.prep.prepare(link / 'new', cache, offline=True)

    def test_module_receipt_cannot_claim_guest_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'out'
            with self.assertRaisesRegex(ValueError, 'guest'):
                self.prep.publish(output, {'modules.cpio': b'x'}, {'preparation': 'VERIFIED', 'guest_status': 'PASS'})
            self.assertFalse(output.exists())


if __name__ == '__main__': unittest.main()
