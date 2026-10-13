"""Strict basket profile preparation; actual guest execution is a later gate."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).parents[1] / 'prepare-rv1103-basket.py'
FIXTURE = Path(__file__).parents[2] / 'examples/luckfox-pico-mini-a-linux/basket'

def elf():
    data = bytearray(256)
    ident = b'\x7fELF\x01\x01\x01' + bytes(9)
    data[:52] = struct.pack('<16sHHIIIIIHHHHHH', ident, 2, 40, 1, 0x10074, 52, 0, 0x05000200, 52, 32, 1, 40, 0, 0)
    data[52:84] = struct.pack('<8I', 1, 0, 0x10000, 0x10000, 256, 256, 5, 0x1000)
    return bytes(data)

class BasketPreparationTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), 'distinct strict basket preparer is required')
        spec = importlib.util.spec_from_file_location('basket_prepare', SCRIPT)
        self.prep = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.prep)

    def entries(self):
        return self.prep.archive_entries(b'busybox', b'init', elf(), {'i2c-dev.ko': b'dev', 'i2c-rk3x.ko': b'rk'})

    def test_one_exact_newc_archive_with_no_extra_or_duplicate_entries(self):
        entries = self.entries(); archive = self.prep.newc(entries)
        self.prep.validate_newc(archive, entries)
        for damaged in (archive + archive, archive[:-1], archive[:6] + b'z' + archive[7:]):
            with self.subTest(damage=len(damaged)), self.assertRaises(ValueError): self.prep.validate_newc(damaged, entries)
        extra = dict(entries); extra['bin/unreviewed'] = (stat.S_IFREG | 0o755, b'x', 0, 0)
        with self.assertRaises(ValueError): self.prep.validate_newc(self.prep.newc(extra), entries)
        # Inject a duplicate actual record before the legitimate next record.
        fields = [int(archive[6+i*8:14+i*8], 16) for i in range(13)]
        end = (110 + fields[11] + 3) & ~3
        end = (end + fields[6] + 3) & ~3
        with self.assertRaises(ValueError): self.prep.validate_newc(archive[:end] + archive, entries)

    def test_archive_checks_modes_symlinks_and_device_numbers(self):
        entries = self.entries()
        for name, change in [('bin/sh', (stat.S_IFLNK | 0o777, b'/outside', 0, 0)), ('init', (stat.S_IFREG | 0o644, b'init', 0, 0)), ('dev/console', (stat.S_IFCHR | 0o600, b'', 1, 3))]:
            changed = dict(entries); changed[name] = change
            with self.subTest(name=name), self.assertRaises(ValueError): self.prep.validate_newc(self.prep.newc(changed), entries)

    def test_archive_and_source_byte_limits(self):
        with self.assertRaises(ValueError): self.prep.validate_newc(bytes(8*1024*1024+1), self.entries())
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp) / 'large'; p.write_bytes(b'a'*1025)
            with self.assertRaises(ValueError): self.prep.read_regular(p, 1024)

    def test_symlink_in_any_input_or_output_parent_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); (root/'real').mkdir(); (root/'real/file').write_bytes(b'ok'); (root/'link').symlink_to(root/'real')
            with self.assertRaises(ValueError): self.prep.read_regular(root/'link/file', 100)
            with self.assertRaises(ValueError): self.prep.base.check_output(root/'link/output')

    def test_helper_rejects_wrong_abi_dynamic_imports_and_unbounded_segments(self):
        self.prep.verify_helper_elf(elf())
        for offset, fmt, value in [(18, '<H', 62), (16, '<H', 3), (36, '<I', 0x04000000), (52, '<I', 2), (52, '<I', 3), (72, '<I', 0x10000000), (24, '<I', 0x90000000)]:
            damaged = bytearray(elf()); struct.pack_into(fmt, damaged, offset, value)
            with self.subTest(offset=offset, value=value), self.assertRaises(ValueError): self.prep.verify_helper_elf(damaged)
        # Real symbol table with undefined 'printf', not an ELF text search.
        damaged = bytearray(elf()); struct.pack_into('<I', damaged, 32, 256); struct.pack_into('<HHH', damaged, 46, 40, 3, 0)
        damaged.extend(bytes(120)); symbols = len(damaged); damaged.extend(bytes(16) + struct.pack('<IIIBBH', 1, 0, 0, 0x12, 0, 0)); strings=len(damaged); damaged.extend(b'\0printf\0')
        struct.pack_into('<10I', damaged, 296, 0, 2, 0, 0, symbols, 32, 2, 1, 4, 16)
        struct.pack_into('<10I', damaged, 336, 0, 3, 0, 0, strings, 8, 0, 0, 1, 0)
        with self.assertRaisesRegex(ValueError, 'undefined|import'): self.prep.verify_helper_elf(bytes(damaged))

    def compile_dt(self, text, temp):
        dts = Path(temp)/'test.dts'; dtb=Path(temp)/'test.dtb'; dts.write_text(text)
        subprocess.run(['dtc','-I','dts','-O','dtb','-o',str(dtb),str(dts)], check=True, capture_output=True, timeout=10)
        return dtb.read_bytes()

    def test_exact_twelve_nodes_clocks_irqs_aliases_and_initrd_range(self):
        text=(FIXTURE/'rv1103-basket.dts').read_text().replace('@INITRD_END@','0x03240000')
        with tempfile.TemporaryDirectory() as temp:
            self.prep.validate_dtb(self.compile_dt(text,temp),0x03240000)
            mutations=[('serial4 = &uart4','serial4 = &uart2'), ('i2c3 = &i2c3','i2c3 = &uart2'), ('<0 21 4>','<0 22 4>'), ('<0 29 4>','<0 30 4>'), ('"i2c", "pclk"','"bad", "pclk"'), ('rockchip,rk3399-i2c','rockchip,rk3288-i2c'), ('clocks = <&clock24m>, <&clock24m>','clocks = <&gic>, <&clock24m>'), ('status = "okay"','status = "okay"; malicious = <1>'), ('0x03240000','0x03240004')]
            for before,after in mutations:
                self.assertIn(before,text)
                with self.subTest(before=before), self.assertRaises(ValueError): self.prep.validate_dtb(self.compile_dt(text.replace(before,after),temp),0x03240000)
            with self.assertRaises(ValueError): self.prep.validate_dtb(self.compile_dt(text,temp),0x04000004)
            # Base ten-node validator stays strict.
            with self.assertRaises(ValueError): self.prep.base.validate_dtb(self.compile_dt(text,temp),0x03240000)

    def test_checked_pins_reject_source_tampering(self):
        pins=self.prep.load_pins(); self.prep.fixture_sources(pins)
        with tempfile.TemporaryDirectory() as temp:
            other=Path(temp); (other/'helper.c').write_bytes(b'tampered')
            with self.assertRaises(ValueError): self.prep.fixture_sources(pins, other)

    def test_manifest_cannot_authenticate_arbitrary_modules(self):
        with tempfile.TemporaryDirectory() as temp:
            folder=Path(temp); (folder/'module-manifest.json').write_text(json.dumps({'artifacts':{'i2c-rk3x.ko':{'sha256':'chosen-by-user'}}}))
            (folder/'i2c-rk3x.ko').write_bytes(b'wrong')
            with self.assertRaises(ValueError): self.prep.reuse_modules(folder,self.prep.load_pins(), b'')

    def test_phase_metadata_is_consumed_by_generated_helper_header(self):
        meta=json.loads((FIXTURE/'phases.json').read_text()); header=self.prep.phase_header(meta)
        self.assertIn(b'256, -128, 64',header); self.assertIn(b'-64, 128, -256',header)
        self.assertEqual([p['id'] for p in meta['phases']],['stimulus_a','stimulus_b','external_off','external_restored'])
        self.assertEqual(meta['ready_wait_ms'],100); self.assertEqual(meta['uart_deadline_ms'],500)

    def test_two_distinct_actual_build_directories_produce_identical_arm_helper(self):
        pins=self.prep.load_pins(); sources=self.prep.fixture_sources(pins)
        with tempfile.TemporaryDirectory() as temp:
            first,receipt_a,log_a=self.prep.build_helper(Path(temp)/'one',sources,pins)
            second,receipt_b,log_b=self.prep.build_helper(Path(temp)/'two',sources,pins)
            self.assertEqual(first,second); self.assertNotEqual(receipt_a['directory'],receipt_b['directory'])
            self.prep.verify_helper_elf(first); self.assertTrue(log_a); self.assertTrue(log_b)
            self.assertEqual(self.prep.modules.identity(first),pins['helper'])

    def test_relative_build_directory_is_supported_without_wrong_cwd_paths(self):
        pins=self.prep.load_pins(); sources=self.prep.fixture_sources(pins)
        with tempfile.TemporaryDirectory() as temp:
            previous=Path.cwd()
            try:
                os.chdir(temp)
                binary,receipt,_=self.prep.build_helper(Path('relative'),sources,pins)
                self.assertEqual(self.prep.modules.identity(binary),pins['helper'])
                self.assertTrue(Path(receipt['directory']).is_absolute())
            finally: os.chdir(previous)

    def test_helper_rejects_unreviewed_abi_flags_and_ident(self):
        for offset,fmt,value in [(36,'<I',0x05000210),(7,'<B',3),(8,'<B',1)]:
            damaged=bytearray(elf()); struct.pack_into(fmt,damaged,offset,value)
            with self.subTest(offset=offset), self.assertRaises(ValueError): self.prep.verify_helper_elf(bytes(damaged))

    def test_oversized_tool_is_rejected_before_any_execution(self):
        pins=self.prep.load_pins(); sources=self.prep.fixture_sources(pins)
        with tempfile.TemporaryDirectory() as temp:
            directory=Path(temp); sentinel=directory/'executed'; tool=directory/'arm-none-eabi-gcc'
            with tool.open('wb') as file:
                file.write(('#!/bin/sh\nprintf attempted > '+str(sentinel)+'\necho "'+pins['compiler_version']+'"\nexit 0\n').encode())
                file.truncate(64*1024*1024+1)
            tool.chmod(0o755); previous=os.environ['PATH']
            try:
                os.environ['PATH']=temp
                with self.assertRaises(ValueError): self.prep.build_helper(directory/'build',sources,pins)
                self.assertFalse(sentinel.exists(),'oversized tool was executed before its byte bound was checked')
            finally: os.environ['PATH']=previous

    def uart_identity_status(self,path,dt):
        init=(FIXTURE/'init').read_text()
        if '# UART identity validator begin' in init:
            validator=init.split('# UART identity validator begin\n',1)[1].split('# UART identity validator end',1)[0]
            script=validator+'\nuart_identity "$1" "$2"\n'
        else:
            # Causal RED executes the current production check, not a copy of
            # the proposed validator. No mounts/modules/guest code is run.
            old=next(line for line in init.splitlines() if line.startswith('case "$uartpath"'))
            script='fail() { return 1; }\nuartpath=$1\n'+old+'\n'
        result=subprocess.run(['/bin/sh','-c',script,'uart-validator',path,dt],capture_output=True,timeout=2)
        return result.returncode

    def taint_status(self, path):
        busybox = shutil.which('busybox')
        if not busybox:
            self.skipTest('native BusyBox is required for the sysctl read regression')
        init = (FIXTURE/'init').read_text()
        lines = init.splitlines()
        start = next(i for i, line in enumerate(lines) if '/proc/sys/kernel/tainted' in line)
        end = next(i for i, line in enumerate(lines) if line.startswith('i2cpath='))
        block = '\n'.join(lines[start:end]).replace('/proc/sys/kernel/tainted', '"$1"')
        # Use the same BusyBox cat applet as the guest, not a host GNU cat.
        script = 'fail() { echo "failure=$1"; exit 77; }\nbusybox=$2\ncat() { "$busybox" cat "$@"; }\n' + block
        return subprocess.run([busybox, 'sh', '-c', script, 'taint-validator', str(path), busybox], capture_output=True, timeout=2)

    def test_taint_read_consumes_complete_native_sysctl_value_with_busybox(self):
        path = Path('/proc/sys/kernel/tainted')
        if not path.is_file():
            self.skipTest('read-only Linux taint sysctl is required')
        expected = path.read_bytes().strip()
        self.assertTrue(expected.isdigit())
        result = self.taint_status(path)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout, b'LW_BASKET_TAINT=' + expected + b'\n')

    def test_taint_read_preserves_format_and_read_failure_guards(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'tainted'
            for bad in (b'', b'-1\n', b'12x\n', b'12\n34\n'):
                path.write_bytes(bad)
                with self.subTest(value=bad):
                    self.assertEqual(self.taint_status(path).returncode, 77)
            path.unlink()
            self.assertEqual(self.taint_status(path).returncode, 77)

    def test_uart_identity_accepts_pinned_kernel_serial_ancestry_with_independent_dt(self):
        physical='/sys/devices/platform/ff4e0000.serial'
        dt='/sys/firmware/devicetree/base/serial@ff4e0000'
        for controller,port in [('0','0'),('3','17'),('12','4')]:
            path=f'{physical}/ff4e0000.serial:{controller}/ff4e0000.serial:{controller}.{port}'
            with self.subTest(path=path): self.assertEqual(self.uart_identity_status(path,dt),0,'source-derived serial-port parent must pass')

    def test_uart_identity_rejects_wrong_dt_controller_or_noncanonical_ancestry(self):
        dt='/sys/firmware/devicetree/base/serial@ff4e0000'
        path='/sys/devices/platform/ff4e0000.serial/ff4e0000.serial:0/ff4e0000.serial:0.0'
        invalid=[(path,dt.replace('ff4e','ff4c')),(path,dt+'/extra'),(path,dt+'\n'),
                 (path.replace('ff4e0000.serial:0.0','ff4e0000.serial:1.0'),dt),
                 (path.replace('ff4e0000.serial:0/','ff4c0000.serial:0/'),dt),
                 (path.replace('/platform/ff4e0000.serial/','/platform/ff4c0000.serial/'),dt),
                 (path+'/tty/ttyS4',dt),(path+'/extra',dt),(path.replace('/platform/','/platform/extra/'),dt),
                 ('/sys/devices/platform/ff4e0000.serial/tty/ttyS4',dt),
                 (path.replace(':0/',':00/').replace(':0.0',':00.0'),dt),
                 (path.replace(':0.0',':0.-1'),dt),(path.replace(':0.0',':0.00'),dt),
                 (path.replace(':0.0',':0.2147483648'),dt),(path.replace(':0.0',':0.1.2'),dt),
                 (path.replace('/platform/','//platform/'),dt),(path+'\n',dt),(path+'\\n',dt),
                 ('x'*513,dt),(path,'x'*513)]
        for bad_path,bad_dt in invalid:
            with self.subTest(path=bad_path,dt=bad_dt): self.assertNotEqual(self.uart_identity_status(bad_path,bad_dt),0)

if __name__=='__main__': unittest.main()
