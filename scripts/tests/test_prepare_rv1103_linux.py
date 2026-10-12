import importlib.util
import gzip
import io
import tarfile
import tempfile
import unittest
from unittest import mock
from pathlib import Path
spec = importlib.util.spec_from_file_location('prepare', Path(__file__).parents[1] / 'prepare-rv1103-linux.py')
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)

def archive(entries):
    b = io.BytesIO()
    with tarfile.open(fileobj=b, mode='w:gz') as tar:
        for name, data, kind in entries:
            m = tarfile.TarInfo(name); m.size = len(data); m.type = kind
            tar.addfile(m, io.BytesIO(data))
    return b.getvalue()

def metadata_archive(kind, size):
    b = io.BytesIO()
    name = 'bin/tool' if kind == 'pax' else 'a' * size
    with tarfile.open(fileobj=b, mode='w:gz', format=tarfile.PAX_FORMAT if kind == 'pax' else tarfile.GNU_FORMAT) as tar:
        m = tarfile.TarInfo(name); m.size = 3
        if kind == 'pax': m.pax_headers = {'comment': 'a' * size}
        tar.addfile(m, io.BytesIO(b'abc'))
    return b.getvalue(), name

class PreparationTests(unittest.TestCase):
    def test_exact_regular_members(self):
        data = archive([('bin/tool', b'abc', tarfile.REGTYPE)])
        self.assertEqual(prepare.selected_members(data, ['bin/tool'], 2, 16, 32), {'bin/tool': b'abc'})
    def test_duplicates_missing_and_links(self):
        for entries in [[('bin/tool', b'a', tarfile.REGTYPE)] * 2, [], [('bin/tool', b'', tarfile.SYMTYPE)], [('../evil', b'a', tarfile.REGTYPE)]]:
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                prepare.selected_members(archive(entries), ['bin/tool'], 3, 16, 32)
    def test_member_and_cumulative_limits(self):
        for limits in [(0, 16, 32), (3, 2, 32), (3, 16, 2)]:
            with self.assertRaises(ValueError): prepare.selected_members(archive([('bin/tool', b'abc', tarfile.REGTYPE)]), ['bin/tool'], *limits)
    def test_expanded_archive_bytes_include_zero_padding(self):
        data = gzip.compress(gzip.decompress(archive([('bin/tool', b'abc', tarfile.REGTYPE)])) + bytes(2 * 1024 * 1024))
        with mock.patch.object(prepare.tarfile, 'open', side_effect=AssertionError('semantic parser ran before physical validation')), self.assertRaisesRegex(ValueError, 'expanded'):
            prepare.selected_members(data, ['bin/tool'], 1, 3, 3)
    def test_physical_count_includes_hidden_extension_records(self):
        for kind in ('pax', 'gnu'):
            data, name = metadata_archive(kind, 128)
            with self.subTest(kind=kind), mock.patch.object(prepare.tarfile, 'open', side_effect=AssertionError('semantic parser ran before physical validation')), self.assertRaisesRegex(ValueError, 'physical.*count'):
                prepare.selected_members(data, [name], 1, 3, 3)
    def test_oversized_pax_and_gnu_metadata_before_tarfile(self):
        for kind in ('pax', 'gnu'):
            data, name = metadata_archive(kind, 65537)
            with self.subTest(kind=kind), mock.patch.object(prepare.tarfile, 'open', side_effect=AssertionError('semantic parser ran before physical validation')), self.assertRaisesRegex(ValueError, 'metadata'):
                prepare.selected_members(data, [name], 4, 3, 3)
    def test_expansion_exact_boundary_and_physical_statistics(self):
        for kind in ('pax', 'gnu'):
            data, name = metadata_archive(kind, 128); size = len(gzip.decompress(data)); stats = {}
            self.assertEqual(prepare.selected_members(data, [name], 1, 3, 3, statistics=stats, expanded_limit=size, physical_count_limit=2, metadata_limit=512, metadata_total_limit=512), {name: b'abc'})
            self.assertEqual((stats['members'], stats['physical_records'], stats['extension_records'], stats['expanded_tar_bytes'], stats['physical_payload_bytes']), (1, 2, 1, size, 3))
            with self.assertRaisesRegex(ValueError, 'expanded'):
                prepare.selected_members(data, [name], 1, 3, 3, expanded_limit=size - 1, physical_count_limit=2)
    def test_cumulative_metadata_and_truncated_physical_payload(self):
        b = io.BytesIO()
        with tarfile.open(fileobj=b, mode='w:gz', format=tarfile.PAX_FORMAT) as tar:
            for name in ('bin/tool', 'bin/other'):
                m = tarfile.TarInfo(name); m.size = 3; m.pax_headers = {'comment': 'a' * 128}; tar.addfile(m, io.BytesIO(b'abc'))
        with mock.patch.object(prepare.tarfile, 'open', side_effect=AssertionError('semantic parser ran before physical validation')), self.assertRaisesRegex(ValueError, 'metadata'):
            prepare.selected_members(b.getvalue(), ['bin/tool'], 4, 3, 6, metadata_limit=512, metadata_total_limit=200)
        header = gzip.decompress(archive([('bin/tool', b'abc', tarfile.REGTYPE)]))[:512]
        with self.assertRaisesRegex(ValueError, 'truncated physical TAR payload'):
            prepare.selected_members(gzip.compress(header), ['bin/tool'], 1, 3, 3)
    def test_recursive_pax_chain_returns_controlled_error(self):
        data = archive([('pax', b'15 comment=abc\n', tarfile.XHDTYPE)] * 1200 + [('bin/tool', b'abc', tarfile.REGTYPE)])
        with self.assertRaisesRegex(ValueError, 'recursion|nesting'):
            prepare.selected_members(data, ['bin/tool'], 1300, 16, 32, expanded_limit=2000000, metadata_limit=512, metadata_total_limit=20000)
    def test_cache_rehashed(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / ('0' * 64 + '.apk'); p.write_bytes(b'wrong')
            with self.assertRaises(ValueError): prepare.cached_apk(Path(d), 'https://invalid.test', '0' * 64, 5, offline=True)
    def test_fresh_download_hash_and_size_before_cache_commit(self):
        with tempfile.TemporaryDirectory() as d:
            source = Path(d) / 'source'; source.write_bytes(b'abc')
            cache = Path(d) / 'cache'
            sha = prepare.digest(b'abc')
            self.assertEqual(prepare.cached_apk(cache, source.as_uri(), sha, 3), b'abc')
            (cache / (sha + '.apk')).write_bytes(b'bad')
            with self.assertRaises(ValueError): prepare.cached_apk(cache, source.as_uri(), sha, 3)
            for expected, size in [('0' * 64, 3), (sha, 2)]:
                other = Path(d) / ('cache' + str(size))
                with self.assertRaises(ValueError): prepare.cached_apk(other, source.as_uri(), expected, size)
                self.assertFalse((other / (expected + '.apk')).exists())

    def test_output_symlink_and_existing_output_refused(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / 'out'; p.symlink_to(Path(d), target_is_directory=True)
            with self.assertRaises(ValueError): prepare.check_output(p)
            p.unlink(); p.mkdir()
            with self.assertRaises(ValueError): prepare.check_output(p)
    def test_newc_is_reproducible_and_has_devices(self):
        a = prepare.initramfs(b'busy', b'init')
        self.assertEqual(a, prepare.initramfs(b'busy', b'init'))
        self.assertIn(b'dev/console\0', a); self.assertIn(b'TRAILER!!!\0', a)
    def test_compiled_dtb_geometry_tamper(self):
        import subprocess
        source = (prepare.EXAMPLE / 'rv1103-direct.dts').read_text().replace('@INITRD_END@', '0x03101000')
        with tempfile.TemporaryDirectory() as d:
            for old, new in [(None, None), ('0x04000000', '0x08000000'), ('rockchip,rv1103', 'rockchip,rv1106'), ('0x03101000', '0x03102000'), ('24000000', '25000000'), ('compatible = "arm,armv7-timer";', 'compatible = "arm,armv7-timer"; status = "disabled";')]:
                text = source if old is None else source.replace(old, new)
                dts = Path(d) / 'test.dts'; dtb = Path(d) / 'test.dtb'; dts.write_text(text)
                subprocess.run(['dtc', '-q', '-I', 'dts', '-O', 'dtb', '-o', str(dtb), str(dts)], check=True)
                if old is None: prepare.validate_dtb(dtb.read_bytes(), 0x03101000)
                else:
                    with self.assertRaises(ValueError): prepare.validate_dtb(dtb.read_bytes(), 0x03101000)

    def test_reserved_phandles_with_matching_references(self):
        import subprocess
        source = (prepare.EXAMPLE / 'rv1103-direct.dts').read_text().replace('@INITRD_END@', '0x03101000')
        for node, reference, prop, values in [('/clock24m', '/serial@ff4c0000', 'clocks', ['ffffffff', 'ffffffff']), ('/interrupt-controller@ff1f1000', '/', 'interrupt-parent', ['ffffffff'])]:
            with self.subTest(node=node), tempfile.TemporaryDirectory() as d:
                dts = Path(d) / 'test.dts'; dtb = Path(d) / 'test.dtb'; dts.write_text(source)
                subprocess.run(['dtc', '-q', '-I', 'dts', '-O', 'dtb', '-o', str(dtb), str(dts)], check=True)
                subprocess.run(['fdtput', '-t', 'x', str(dtb), node, 'phandle', 'ffffffff'], check=True)
                subprocess.run(['fdtput', '-t', 'x', str(dtb), reference, prop, *values], check=True)
                with self.assertRaisesRegex(ValueError, 'phandle'):
                    prepare.validate_dtb(dtb.read_bytes(), 0x03101000)

    def test_pkg_identity_and_config(self):
        with self.assertRaises(ValueError): prepare.verify_identity(b'pkgname = wrong\npkgver = 1\narch = armv7\n', 'right', '1')
        with self.assertRaises(ValueError): prepare.verify_config(b'CONFIG_ARM_LPAE=y\n')

if __name__ == '__main__': unittest.main()
