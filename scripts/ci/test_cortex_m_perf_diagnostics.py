"""Synthetic diagnostic controls; no firmware build, Valgrind or simulator."""

import hashlib
import ast
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
PATH = ROOT / "scripts/ci/cortex_m_perf_diagnostics.py"


def elf_bytes(debug=b"base-debug", *, address=0x08000000):
    data = bytearray(120)
    data[:7] = b"\x7fELF\x01\x01\x01"
    struct.pack_into("<HHIIIIIHHHHHH", data, 16, 2, 40, 1, address + 1,
                     52, 0, 0x05000000, 52, 32, 1, 40, 0, 0)
    struct.pack_into("<IIIIIIII", data, 52, 1, 116, address, address, 4, 8, 5, 4)
    data[116:120] = b"\x00\xbf\xfe\xe7"
    return bytes(data) + debug


def annotation(total=10):
    return ("Events recorded:  Ir\nEvents shown:     Ir\n"
            f"{total}  PROGRAM TOTALS\n\nIr  file:function\n"
            "-------------------\n7  src/a.rs:start [cli]\n3  src/b.rs:spin [cli]\n")


class DiagnosticsControls(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("diagnostics", PATH)
        if PATH.exists():
            self.diag = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(self.diag)
        else:
            self.diag = types.SimpleNamespace(inspect_elf=lambda *a: None,
                                              parse_annotation=lambda *a: None)
        spec = importlib.util.spec_from_file_location("perf", ROOT / "scripts/ci/cortex_m_perf_evidence.py")
        self.perf = importlib.util.module_from_spec(spec); spec.loader.exec_module(self.perf)
        self.temp = tempfile.TemporaryDirectory(prefix="cortex-diagnostic-controls-")
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)

    def elf(self, name="firmware", data=None):
        path = self.path / name; path.write_bytes(elf_bytes() if data is None else data)
        return path

    def test_real_load_descriptor_preserves_full_hash_and_zero_fill(self):
        path = self.elf(); proof = self.diag.inspect_elf(path)
        self.assertIsNotNone(proof, "missing real ELF load descriptor")
        self.assertEqual(proof["sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
        self.assertEqual(proof["image"]["entry"], 0x08000001)
        self.assertEqual(proof["image"]["segments"][0]["zero_fill"], 4)
        self.assertEqual(proof["image"]["segments"][0]["permissions"], 5)

    def test_debug_difference_is_not_a_normalized_full_file_hash(self):
        a = self.diag.inspect_elf(self.elf("a", elf_bytes(b"source-base")))
        b = self.diag.inspect_elf(self.elf("b", elf_bytes(b"source-head")))
        self.assertIsNotNone(a, "missing ELF proof")
        self.assertEqual(a["image"], b["image"])
        self.assertNotEqual(a["sha256"], b["sha256"])

    def test_each_loaded_image_semantic_difference_is_detected(self):
        original = elf_bytes(); reference = self.diag.inspect_elf(self.elf("base", original))
        self.assertIsNotNone(reference, "missing ELF proof")
        for offset, value in ((24,0x08000003),(36,0x04000000),(60,0x08000004),
                              (64,0x08000004),(72,12),(76,7),(80,8),(116,0x1234)):
            data = bytearray(original); struct.pack_into("<I", data, offset, value)
            with self.subTest(offset=offset):
                try:
                    changed = self.diag.inspect_elf(self.elf("changed",data))
                except ValueError:
                    continue
                self.assertNotEqual(reference["image"], changed["image"])

    def test_malformed_truncated_overlarge_and_unbounded_elf_fail(self):
        cases = [b"", b"not ELF", elf_bytes()[:70], elf_bytes()+b"x"*524288]
        for offset, fmt, value in ((4,"B",2),(5,"B",2),(18,"H",62),(42,"H",0),
                                   (44,"H",0),(44,"H",65),(56,"I",99999),
                                   (68,"I",9),(72,"I",0xffffffff),(76,"I",8)):
            data=bytearray(elf_bytes());struct.pack_into("<"+fmt,data,offset,value);cases.append(data)
        for data in cases:
            with self.subTest(length=len(data)), self.assertRaises(ValueError):
                self.diag.inspect_elf(self.elf("bad",data))

    def test_annotation_requires_actual_self_costs_and_equal_raw_total(self):
        proof = self.diag.parse_annotation(annotation().encode(), 10)
        self.assertIsNotNone(proof, "missing function-cost evidence")
        self.assertEqual(proof["total_ir"],10)
        self.assertEqual([r["ir"] for r in proof["functions"]],[7,3])
        self.assertEqual(proof["self_cost_sum"],10)

    def test_annotation_fakepass_inconsistent_missing_or_oversize_fails(self):
        good=annotation()
        cases=[b"",good.replace('10  PROGRAM TOTALS','11  PROGRAM TOTALS').encode(),
               good.replace('PROGRAM TOTALS','PROGRAM TOTALS (calculated)').encode(),
               good.replace('3  src/b.rs:spin','2  src/b.rs:spin').encode(),
               good.replace('Ir  file:function','X  file:function').encode(),
               good.replace('3  src/b.rs:spin','3  src/a.rs:start').encode(),
               good.replace('10  PROGRAM TOTALS','1,,0  PROGRAM TOTALS').encode(),
               (good+'x'*65536).encode()]
        for data in cases:
            with self.subTest(length=len(data)), self.assertRaises(ValueError):
                self.diag.parse_annotation(data,10)

    def test_overlapping_load_ranges_are_not_accepted(self):
        data=bytearray(elf_bytes()[:116]+b'\0'*40)
        struct.pack_into('<H',data,44,2)
        struct.pack_into('<IIIIIIII',data,52,1,148,0x08000000,0x08000000,4,8,5,4)
        struct.pack_into('<IIIIIIII',data,84,1,152,0x08000004,0x08000004,4,8,6,4)
        data[148:156]=b'\0\xbf\xfe\xe7'*2
        with self.assertRaises(ValueError):self.diag.inspect_elf(self.elf('overlap',data))

    def modules(self, changed=False, skipped=False):
        modules={};sources={};calls=[]
        for side in ('base','head'):
            source=self.path/side;source.mkdir();sources[side]=source
            paths={}
            for name,address in (('nrf',0),('stm32',0x08000000)):
                path=source/('perf-spin-'+name)
                data=elf_bytes(side.encode(),address=address)
                if changed and side=='head' and name=='stm32':
                    data=bytearray(data);data[116]=1
                path.write_bytes(data);paths[name]=path
            def build(names,side=side,paths=paths):
                calls.append((side,set(names),os.getcwd()))
                return paths, {'nrf':'missing toolchain'} if skipped else {}
            modules[side]=types.SimpleNamespace(build_fixtures=build,subprocess=subprocess,
                discover_chips=lambda:{},plan_coverage=lambda _:({'nrf52840':'nrf','stm32f405':'stm32','stm32l476':'stm32'},{}),
                fixture_for=lambda chip:chip,
                fixture_origins=lambda name:(0 if name=='nrf' else 0x08000000,0x20000000))
        return modules,sources,calls

    def prepare(self,**kwargs):
        self.assertTrue(hasattr(self.diag,'prepare_common_fixtures'),"missing common-input protocol")
        modules,sources,calls=self.modules(**kwargs)
        output=self.path/'evidence';output.mkdir()
        common=self.diag.prepare_common_fixtures(modules,sources,output,self.perf.Budget(),self.perf.BoundedRunner)
        return common,modules,sources,calls,output

    def test_original_builders_run_once_each_and_four_originals_retained(self):
        common,modules,sources,calls,out=self.prepare()
        self.assertEqual([c[:2] for c in calls],[('base',{'nrf','stm32'}),('head',{'nrf','stm32'})])
        self.assertEqual([c[2] for c in calls],[str(sources['base']),str(sources['head'])])
        self.assertEqual(len(list((out/'fixtures').glob('*/*.elf'))),6)
        self.assertEqual(common.evidence['originals']['base']['nrf']['image'],common.evidence['originals']['head']['nrf']['image'])
        self.assertNotEqual(common.evidence['originals']['base']['nrf']['sha256'],common.evidence['originals']['head']['nrf']['sha256'])
        for path in common.paths.values():
            self.assertEqual(path.stat().st_mode & 0o222,0)
        self.assertIs(modules['base'].subprocess,subprocess)

    def test_actual_original_build_function_keeps_argv_origin_and_provenance(self):
        import shutil
        node=next(n for n in ast.parse((ROOT/'scripts/perf/board_perf.py').read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='build_fixtures')
        modules={};sources={}
        for side in ('base','head'):
            source=self.path/side;source.mkdir();sources[side]=source
            module=types.ModuleType('fixture_'+side)
            spec=types.SimpleNamespace(builder='cargo',toolchain=None,target='thumbv6m-none-eabi',crate='firmware-perf-spin',directory=None,features=None,env_origins=True,optional=False)
            module.__dict__.update(Path=Path,sys=sys,os=os,shutil=shutil,REPO_ROOT=source,FIXTURE_DIR=source/'target/perf-fixtures',subprocess=subprocess,
                fixture_spec=lambda _:spec,toolchain_available=lambda _:True,
                fixture_origins=lambda name:(0 if name=='nrf' else 0x08000000,0x20000000),
                _path_with_xtensa_linker=lambda target,path:path,
                discover_chips=lambda:{},plan_coverage=lambda _:({'nrf52840':'nrf','stm32f405':'stm32','stm32l476':'stm32'},{}))
            exec(compile(ast.Module(body=[node],type_ignores=[]),'original-build-fixtures','exec'),module.__dict__)
            modules[side]=module
        out=self.path/'evidence';out.mkdir();calls=[]
        def compiler_control(command,**kwargs):
            self.assertEqual(command,['cargo','build','--release','--target','thumbv6m-none-eabi','-p','firmware-perf-spin'])
            env=kwargs['env'];source=Path(kwargs['cwd']);flash=int(env['LABWIRED_PERF_FLASH_ORIGIN'],16)
            self.assertEqual(env['LABWIRED_PERF_RAM_ORIGIN'],'0x20000000')
            path=source/'target/thumbv6m-none-eabi/release/firmware-perf-spin';path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes(elf_bytes(source.name.encode(),address=flash));calls.append((source.name,flash))
            result=subprocess.CompletedProcess(command,0,b'synthetic compiler receipt',b'');result.producer_exit_code=0;return result
        with mock.patch.object(self.perf,'collect',side_effect=compiler_control):
            common=self.diag.prepare_common_fixtures(modules,sources,out,self.perf.Budget(),self.perf.BoundedRunner)
        self.assertEqual(calls,[('base',0),('base',0x08000000),('head',0),('head',0x08000000)])
        for side in ('base','head'):
            self.assertEqual(len(common.evidence['build_commands'][side]),2)
            self.assertIs(modules[side].subprocess,subprocess)
            self.assertTrue((out/f'fixture-build/{side}-child-01.json').exists())
        self.assertEqual(common.evidence['originals']['base']['nrf']['image']['entry'],1)

    def test_changed_loaded_image_or_skipped_build_does_not_freeze_common_pair(self):
        for kwargs in ({'changed':True},{'skipped':True}):
            with self.subTest(kwargs=kwargs):
                with tempfile.TemporaryDirectory() as tmp:
                    self.path=Path(tmp)
                    with self.assertRaises(ValueError):self.prepare(**kwargs)
                    self.assertFalse((self.path/'evidence/fixtures/common').exists())

    def test_scoped_builder_replacement_restores_original_even_on_error(self):
        common,modules,sources,calls,out=self.prepare();module=modules['head'];original=module.build_fixtures
        with self.assertRaisesRegex(ValueError,'injected'):
            with common.install(module):
                paths,skipped=module.build_fixtures({'nrf','stm32'})
                self.assertEqual(paths,common.paths);self.assertFalse(skipped)
                with self.assertRaises(ValueError):module.build_fixtures({'nrf'})
                raise ValueError('injected')
        self.assertIs(module.build_fixtures,original)
        self.assertEqual(len(calls),2)

    def test_whole_file_mutation_and_wrong_path_are_rejected(self):
        common,modules,sources,calls,out=self.prepare()
        path=common.paths['nrf'];before=common.verify(path)
        self.assertEqual(before['sha256'],hashlib.sha256(path.read_bytes()).hexdigest())
        with self.assertRaises(ValueError):common.verify(sources['head']/'perf-spin-nrf')
        path.chmod(0o600);path.write_bytes(path.read_bytes()+b'x');path.chmod(0o444)
        with self.assertRaises(ValueError):common.verify(path)

    def test_original_measure_once_keeps_argv_and_annotates_before_temp_cleanup(self):
        self.assertTrue(hasattr(self.diag,'annotate_call'),'missing pre-cleanup diagnostic hook')
        import re
        from collections import namedtuple
        node=next(n for n in ast.parse((ROOT/'scripts/perf/board_perf.py').read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='measure_once')
        common,modules,sources,calls,out=self.prepare()
        paths=[];commands=[]
        module=types.SimpleNamespace(IREFS_RE=re.compile(r'^==\d+== I refs: ([\d,]+)',re.M),
            BATCHED_RE=re.compile(r'^\[batched\] instructions=(\d+) batches=(\d+) steps_per_batch=([\d.]+) tick_interval=(\d+)',re.M))
        runner=self.perf.BoundedRunner(module,'base',out,[],self.perf.Budget(),common)
        def produce(command,**kwargs):
            commands.append(command)
            path=Path(next(a.split('=',1)[1] for a in command if a.startswith('--callgrind-out-file=')))
            path.write_text('synthetic raw');paths.append(path)
            stderr=b'==1== I refs: 10\n[batched] instructions=200000 batches=200 steps_per_batch=1000 tick_interval=1024\n'
            result=subprocess.CompletedProcess(command,0,b'',stderr);result.producer_exit_code=0;return result
        def annotate(command,ir,prefix,*args):
            path=Path(next(a.split('=',1)[1] for a in command if a.startswith('--callgrind-out-file=')))
            self.assertTrue(path.is_file(),'temporary file already deleted');self.assertEqual(ir,10)
            return dict(total_ir=10,function_count=2)
        cli=self.elf('cli')
        namespace=dict(Path=Path,Run=namedtuple('Run','irefs instructions steps_per_batch tick_interval',defaults=(None,None,None)),
            os=os,tempfile=tempfile,subprocess=runner,MODE_BATCH='batch',MODE_STEP='step',
            IREFS_RE=module.IREFS_RE,BATCHED_RE=module.BATCHED_RE,ModeNotTakenError=ValueError,
            MIN_RECOMMENDED_TICK_INTERVAL=1024)
        exec(compile(ast.Module(body=[node],type_ignores=[]),'original-measure-once','exec'),namespace)
        with mock.patch.object(self.perf,'collect',side_effect=produce),mock.patch.object(self.diag,'annotate_call',side_effect=annotate),mock.patch.dict(sys.modules,cortex_m_perf_diagnostics=self.diag):
            result=namespace['measure_once'](cli,Path('configs/chips/stm32l476.yaml'),common.paths['stm32'],200000,'batch')
        self.assertEqual(result.irefs,10);self.assertFalse(paths[0].exists())
        self.assertEqual(commands[0][:2],['valgrind','--tool=callgrind'])
        self.assertEqual(commands[0][3:],['--cache-sim=no','--branch-sim=no',str(cli),'run','--chip','configs/chips/stm32l476.yaml','--firmware',str(common.paths['stm32']),'--max-steps','200000','--batched'])
        self.assertEqual(runner.rows[0]['fixture_before_sha256'],runner.rows[0]['fixture_after_sha256'])

    def test_mutating_failed_producer_retains_original_logs_and_exit(self):
        import re
        common,modules,sources,calls,out=self.prepare()
        module=types.SimpleNamespace(IREFS_RE=re.compile('unused'),BATCHED_RE=re.compile('unused'))
        runner=self.perf.BoundedRunner(module,'base',out,[],self.perf.Budget(),common)
        fixture=common.paths['nrf']
        def produce(command,**kwargs):
            fixture.chmod(0o600);fixture.write_bytes(fixture.read_bytes()+b'mutation');fixture.chmod(0o444)
            result=subprocess.CompletedProcess(command,7,b'original stdout',b'original stderr');result.producer_exit_code=7;return result
        cmd=['valgrind','cli','run','--chip','configs/chips/nrf52840.yaml','--firmware',str(fixture),'--max-steps','200000']
        with mock.patch.object(self.perf,'collect',side_effect=produce),self.assertRaisesRegex(ValueError,'exit=7'):
            runner.run(cmd,capture_output=True,text=True,env={'LABWIRED_ARM_SINGLE_STEP':'1'})
        self.assertEqual((out/'base-child-01.stdout').read_bytes(),b'original stdout')
        child=json.loads((out/'base-child-01.json').read_text());self.assertEqual(child['exit_code'],7)
        self.assertIn('fixture_validation_error',child)

    def test_shared_retained_budget_fails_without_unbounded_fixture_copy(self):
        modules,sources,calls=self.modules();out=self.path/'evidence';out.mkdir()
        with self.assertRaises(ValueError):
            self.diag.prepare_common_fixtures(modules,sources,out,self.perf.Budget(100),self.perf.BoundedRunner)
        self.assertFalse((out/'fixtures/common').exists())

    def test_exhausted_collector_retains_original_exit_and_fixture_proof(self):
        common,modules,sources,calls,out=self.prepare()
        runner=self.perf.BoundedRunner(types.SimpleNamespace(),'base',out,[],self.perf.Budget(10000),common)
        fixture=common.paths['nrf']
        def exhaust(command,**kwargs):
            kwargs['budget'].remaining=0
            result=subprocess.CompletedProcess(command,125,b'capped stdout',b'capped stderr')
            result.producer_exit_code=-15
            return result
        cmd=['valgrind','cli','run','--chip','configs/chips/nrf52840.yaml','--firmware',str(fixture),'--max-steps','200000']
        with mock.patch.object(self.perf,'collect',side_effect=exhaust),self.assertRaisesRegex(ValueError,'exit=125'):
            runner.run(cmd,capture_output=True)
        child=json.loads((out/'base-child-01.json').read_text())
        self.assertEqual((child['exit_code'],child['producer_exit_code']),(125,-15))
        self.assertEqual(child['fixture_before'],child['fixture_after'])
        self.assertEqual((out/'base-child-01.stdout').read_bytes(),b'capped stdout')
        self.assertEqual((out/'base-child-01.stderr').read_bytes(),b'capped stderr')

    def test_insufficient_metadata_budget_rejects_before_producer(self):
        common,modules,sources,calls,out=self.prepare()
        runner=self.perf.BoundedRunner(types.SimpleNamespace(),'base',out,[],self.perf.Budget(1),common)
        cmd=['valgrind','cli','run','--chip','configs/chips/nrf52840.yaml','--firmware',str(common.paths['nrf']),'--max-steps','200000']
        result=subprocess.CompletedProcess(cmd,125,b'',b'');result.producer_exit_code=-15
        with mock.patch.object(self.perf,'collect',return_value=result) as collect,self.assertRaises(ValueError):
            runner.run(cmd,capture_output=True)
        collect.assert_not_called()

    def test_builder_exception_restores_cwd_argv_environment_and_subprocess(self):
        modules,sources,calls=self.modules();out=self.path/'evidence';out.mkdir()
        before=(os.getcwd(),sys.argv[:],dict(os.environ),modules['base'].subprocess)
        def failed(names):
            os.environ['DIAGNOSTIC_CONTROL_MARKER']='private fixture';sys.argv=['mutated']
            raise RuntimeError('original builder failure')
        modules['base'].build_fixtures=failed
        with self.assertRaisesRegex(RuntimeError,'original builder failure'):
            self.diag.prepare_common_fixtures(modules,sources,out,self.perf.Budget(),self.perf.BoundedRunner)
        self.assertEqual((os.getcwd(),sys.argv,dict(os.environ),modules['base'].subprocess),before)

    def test_annotation_overflow_retains_bounded_output_and_failure_status(self):
        executable=self.path/'callgrind_annotate'
        executable.write_text('#!/usr/bin/env python3\nimport os\nos.write(1,b"x"*100000)\n');executable.chmod(0o755)
        raw=self.path/'cg.out';raw.write_text('synthetic input')
        with mock.patch.dict(os.environ,PATH=str(self.path)+os.pathsep+os.environ['PATH']),self.assertRaises(ValueError):
            self.diag.annotate_call(['valgrind','--callgrind-out-file='+str(raw)],10,self.path/'failed',self.perf.collect,self.perf.Budget(),self.perf.sha256)
        self.assertLessEqual((self.path/'failed.callgrind.stdout').stat().st_size,65536)
        metadata=self.path/'failed.callgrind.json'
        self.assertTrue(metadata.exists(),'annotation failure lost collector metadata')
        self.assertEqual(json.loads(metadata.read_text())['exit_code'],125)

    def test_missing_or_sparse_oversized_callgrind_input_never_starts_annotator(self):
        for huge in (False,True):
            raw=self.path/'input'
            if huge:
                with raw.open('wb') as stream:stream.truncate(64*1024*1024+1)
            with mock.patch.object(self.perf,'collect') as collect,self.assertRaises(ValueError):
                self.diag.annotate_call(['valgrind','--callgrind-out-file='+str(raw)],10,self.path/'owned',collect,self.perf.Budget(),self.perf.sha256)
            collect.assert_not_called()

    def test_official_annotation_handles_compressed_names_and_call_edges(self):
        import shutil
        if shutil.which('callgrind_annotate') is None:self.skipTest('official annotator absent; no parser proof')
        path=self.path/'cg.out'
        path.write_text('''# callgrind format\nversion: 1\ncreator: callgrind-3.22\npid: 123\ncmd: synthetic\npositions: line\nevents: Ir\nsummary: 10\nob=(1) /synthetic/cli\nfl=(1) src/a.rs\nfn=(1) start\n1 7\ncfl=(2) src/b.rs\ncfn=(2) spin\ncalls=1 1\n1 3\nfl=(2)\nfn=(2)\n1 3\ntotals: 10\n''')
        proof=self.diag.annotate_call(['valgrind','--callgrind-out-file='+str(path)],10,self.path/'owned',self.perf.collect,self.perf.Budget(),self.perf.sha256)
        self.assertEqual(proof['total_ir'],10);self.assertEqual(proof['function_count'],2)
        self.assertEqual(proof['input_sha256'],hashlib.sha256(path.read_bytes()).hexdigest())

    def test_defaults_workflow_tool_and_strict_guards_are_unchanged(self):
        original=subprocess.check_output(['git','show','393b5310:scripts/ci/cortex_m_perf_evidence.py'],cwd=ROOT,text=True)
        current=(ROOT/'scripts/ci/cortex_m_perf_evidence.py').read_text()
        def functions(source):
            return {n.name:ast.dump(n,include_attributes=False) for n in ast.parse(source).body if isinstance(n,ast.FunctionDef)}
        old,new=functions(original),functions(current)
        for name in ('collect','validate_identity','validate_records','validate_status','source_contract'):
            self.assertEqual(old[name],new[name],name)
        for path in ('.github/workflows/core-identity.yml','scripts/perf/board_perf.py','scripts/perf/baselines.json'):
            self.assertEqual((ROOT/path).read_bytes(),subprocess.check_output(['git','show','393b5310:'+path],cwd=ROOT))
        self.assertIn('action="store_true"',current)
        self.assertIn('budget = Budget(5 * 1024 * 1024)',current)

    def test_real_optional_wrapper_completes_all_48_with_common_inputs(self):
        import shutil
        if shutil.which('callgrind_annotate') is None:self.skipTest('official annotator absent; no complete route proof')
        from test_cortex_m_perf_evidence import PerformanceControls
        fixture=PerformanceControls();fixture.path=self.path
        sources,binpath=fixture.synthetic_sources()
        identity=fixture.identity()
        for side,(source,_) in sources.items():
            tool=source/'scripts/perf/board_perf.py'
            text=tool.read_text().replace('import os,re,subprocess,sys,json','import os,re,subprocess,sys,json,tempfile')
            text=text.replace('def main():','''def build_fixtures(names):
    assert names=={'nrf','stm32'}
    with Path('original-builder-called').open('a') as marker:marker.write('built\\n')
    return {name:Path.cwd()/('perf-spin-'+name) for name in names},{}
def discover_chips():return {}
def plan_coverage(_):return {'nrf52840':'nrf','stm32f405':'stm32','stm32l476':'stm32'},{}
def fixture_origins(name):return (0 if name=='nrf' else 0x08000000,0x20000000)
def main():''')
            text=text.replace("measured={}\n","measured={}\n    firmware,_=build_fixtures({'nrf','stm32'})\n")
            text=text.replace("'fixture.elf'","str(firmware['nrf' if board=='nrf52840' else 'stm32'])")
            text=text.replace("subprocess.run(cmd,capture_output=True,text=True,env=env)","with tempfile.TemporaryDirectory() as temp:\n                        cmd[1:1]=['--tool=callgrind','--callgrind-out-file='+temp+'/cg.out','--cache-sim=no','--branch-sim=no']\n                        subprocess.run(cmd,capture_output=True,text=True,env=env)")
            tool.write_text(text)
            (source/'Cargo.toml').write_text('[workspace]\n')
            for name,address in (('nrf',0),('stm32',0x08000000)):
                (source/('perf-spin-'+name)).write_bytes(elf_bytes(side.encode(),address=address))
            subprocess.run(['git','add','.'],cwd=source,check=True,capture_output=True)
            subprocess.run(['git','-c','user.name=Fixture','-c','user.email=fixture@example.invalid','-c','core.hooksPath=/dev/null','-c','commit.gpgsign=false','commit','-qm','diagnostic fixture'],cwd=source,check=True,capture_output=True)
            sha=subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()
            sources[side]=(source,sha);(identity/f'identity-{side}/sha').write_text(sha)
        producer=binpath/'valgrind'
        producer.write_text(producer.read_text()+'''
from pathlib import Path
ir=count*(2 if batch else 100)+1000
file=Path(next(a.split('=',1)[1] for a in sys.argv if a.startswith('--callgrind-out-file=')))
file.write_text('version: 1\\npositions: line\\nevents: Ir\\nsummary: '+str(ir)+'\\nfl=fixture.rs\\nfn=loop\\n1 '+str(ir-1000)+'\\nfn=startup\\n1 1000\\ntotals: '+str(ir)+'\\n')
''')
        args=types.SimpleNamespace(base_dir=str(sources['base'][0]),head_dir=str(sources['head'][0]),base_sha=sources['base'][1],head_sha=sources['head'][1],identity_dir=str(identity),steps='5000000 40000000',suite='cortex-m',output=str(self.path/'evidence'),diagnose_common_fixtures=True)
        before=(sys.argv[:],os.getcwd(),dict(os.environ))
        with mock.patch.dict(os.environ,PATH=str(binpath)+os.pathsep+os.environ['PATH']):
            self.assertEqual(self.perf.qualify(args),0)
        self.assertEqual((sys.argv,os.getcwd(),dict(os.environ)),before)
        evidence=json.loads((self.path/'evidence/qualification.json').read_text())
        self.assertEqual(len(evidence['raw_invocations']),48)
        annotations=[r for r in evidence['raw_invocations'] if 'callgrind_annotation' in r]
        self.assertEqual(len(annotations),12)
        self.assertEqual(sum(p.stat().st_size for p in (self.path/'evidence').rglob('*') if p.is_file())<8*1024*1024,True)
        for side,(source,_) in sources.items():
            self.assertEqual((source/'original-builder-called').read_text(),'built\n')
        for row in evidence['raw_invocations']:
            self.assertEqual(row['fixture_sha256'],row['fixture_before_sha256'])
            self.assertEqual(row['fixture_sha256'],row['fixture_after_sha256'])
            self.assertTrue(Path(row['argv'][row['argv'].index('--firmware')+1]).is_relative_to(self.path/'evidence/fixtures/common'))


if __name__ == '__main__':
    unittest.main()
