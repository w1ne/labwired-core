"""Raw-profile retention controls only; no simulator/Valgrind/annotation."""
import ast
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock
from test_cortex_m_perf_diagnostics import elf_bytes
from test_cortex_m_perf_evidence import records

ROOT=Path(__file__).resolve().parents[2]
PATH=ROOT/'scripts/ci/cortex_m_raw_profiles.py'

class RawProfileControls(unittest.TestCase):
    def setUp(self):
        self.raw=types.SimpleNamespace()
        if PATH.exists():
            spec=importlib.util.spec_from_file_location('raw_profiles',PATH)
            self.raw=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.raw)
        self.assertTrue(hasattr(self.raw,'RawProfiles'),'missing explicit immutable raw-profile protocol')
        spec=importlib.util.spec_from_file_location('perf',ROOT/'scripts/ci/cortex_m_perf_evidence.py')
        self.perf=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.perf)
        self.temp=tempfile.TemporaryDirectory(prefix='cortex-raw-controls-');self.addCleanup(self.temp.cleanup);self.path=Path(self.temp.name)
        self.tools={name:self.path/name for name in ('annotator','perl','valgrind','dpkg')}
        for name,path in self.tools.items():path.write_bytes((name+' synthetic official bytes').encode());path.chmod(0o755)
        self.patch=mock.patch.multiple(self.raw,TOOL_PATHS=self.tools,ANNOTATOR_SHA256=hashlib.sha256(self.tools['annotator'].read_bytes()).hexdigest())
        self.patch.start();self.addCleanup(self.patch.stop)
        self.which=mock.patch.object(self.raw.shutil,'which',return_value=str(self.tools['valgrind']));self.which.start();self.addCleanup(self.which.stop)
        self.calls=[]
        def versions(argv,**kwargs):
            self.calls.append(argv)
            response={'/usr/bin/dpkg-query':b'1:3.22.0-0ubuntu3\n','/usr/bin/valgrind':b'valgrind-3.22.0\n','/usr/bin/perl':b'v5.38.2\nx86_64-linux-gnu-thread-multi\n'}[argv[0]]
            result=subprocess.CompletedProcess(argv,0,response,b'');result.producer_exit_code=0;return result
        self.version_collect=versions
    def prepare(self,budget=None):
        output=self.path/'output';output.mkdir(exist_ok=True)
        self.active_raw=self.raw.RawProfiles(output,budget or self.perf.Budget(5*1024*1024),self.version_collect)
        return self.active_raw
    def row(self,repeat=0,ir=10):
        row=next(copy.deepcopy(x) for x in records() if x['side']=='base' and x['board']=='stm32l476' and x['mode']=='batch' and x['steps']==200000 and x['repeat']==repeat)
        row['ir']=ir;return row
    def source(self,row,extra=b''):
        p=self.path/'owned-temp'/f'r{row["repeat"]}'/'cg.out';p.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
        cmd=' '.join(row['argv'][row['argv'].index('run')-1:])
        p.write_bytes(f'version: 1\ncreator: callgrind-3.22.0\ncmd: {cmd}\nevents: Ir\nsummary: {row["ir"]}\nfn=(1) synthetic\n1 {row["ir"]}\n'.encode()+extra)
        command=['valgrind',f'--callgrind-out-file={p}','--cache-sim=no','--branch-sim=no',*row['argv'][1:]]
        row['argv']=command
        if hasattr(self,'active_raw'):
            def measure_once():
                tmp=str(p.parent)
                self.active_raw.preflight(command,os.environ,module,sys._getframe())
            module=types.SimpleNamespace(measure_once=measure_once)
            measure_once()
        return p,command
    def test_three_distinct_profiles_survive_exact_copy_and_guard_still_fails(self):
        raw=self.prepare();allrows=records()
        for repeat,ir in enumerate((10,11,12)):
            row=self.row(repeat,ir);source,cmd=self.source(row,b'#'+b'x'*703000+b'\n');before=source.read_bytes()
            result=raw.capture(cmd,row)
            self.assertEqual(Path(result['path']).read_bytes(),before);source.unlink()
            next(x for x in allrows if (x['side'],x['board'],x['mode'],x['steps'],x['repeat'])==('base','stm32l476','batch',200000,repeat))['ir']=ir
        self.assertTrue(raw.finish()['complete']);self.assertEqual(len(raw.report['profiles']),3)
        with self.assertRaisesRegex(ValueError,'UNSTABLE'):self.perf.validate_records(allrows,1024)
        self.assertEqual(raw.report['qualified'],False)
    def test_semantic_selection_only_three_base_low_batch_repeats(self):
        raw=self.prepare();selected=[]
        for row in records():
            source,cmd=self.source(row)
            if raw.capture(cmd,row):selected.append((row['side'],row['board'],row['mode'],row['steps'],row['repeat']))
        self.assertEqual(selected,[('base','stm32l476','batch',200000,r) for r in range(3)])
        self.assertTrue(raw.finish()['complete'])
    def test_duplicate_missing_fourth_and_final_tamper_reject(self):
        raw=self.prepare();row=self.row();source,cmd=self.source(row);proof=raw.capture(cmd,row)
        with self.assertRaises(ValueError):raw.capture(cmd,row)
        with self.assertRaises(ValueError):raw.finish()
        row=self.row(2);row['repeat']=3;source,cmd=self.source(row)
        with self.assertRaises(ValueError):raw.capture(cmd,row)
        Path(proof['path']).chmod(0o600);Path(proof['path']).write_bytes(b'changed')
        with self.assertRaises(ValueError):raw.verify()
        self.assertFalse(raw.report['complete'])
    def test_size_symlink_header_event_summary_and_duplicate_path_reject(self):
        for mutate in ('size','symlink','header','events','summary','duplicate'):
            with self.subTest(mutate=mutate),tempfile.TemporaryDirectory() as directory:
                old=self.path;self.path=Path(directory);raw=self.prepare();row=self.row();source,cmd=self.source(row)
                if mutate=='size':source.write_bytes(b'x'*(1048576+1))
                elif mutate=='symlink':target=source.with_suffix('.target');source.rename(target);source.symlink_to(target)
                elif mutate=='duplicate':cmd.append(cmd[1])
                else:
                    b=source.read_bytes().replace({'header':b'cmd:','events':b'events: Ir','summary':b'summary: 10'}[mutate],{'header':b'wrong:','events':b'events: Dr','summary':b'summary: 11'}[mutate]);source.write_bytes(b)
                with self.assertRaises((ValueError,OSError)):raw.capture(cmd,row)
                self.assertFalse(raw.report['complete']);self.path=old
    def test_existing_destination_and_midcopy_mutation_preserve_prior_checkpoint(self):
        raw=self.prepare();row=self.row();source,cmd=self.source(row);first=raw.capture(cmd,row)
        row=self.row(1);source,cmd=self.source(row)
        target=raw.directory/'base-stm32l476-batch-200000-r1.callgrind';target.write_bytes(b'protected')
        with self.assertRaises(FileExistsError):raw.capture(cmd,row)
        self.assertEqual(target.read_bytes(),b'protected');self.assertEqual(len(raw.report['profiles']),1)
        target.unlink()
        original=os.fsync
        def mutate(fd):source.write_bytes(source.read_bytes()+b'mutated');original(fd)
        with mock.patch.object(self.raw.os,'fsync',side_effect=mutate),self.assertRaises(ValueError):raw.capture(cmd,row)
        self.assertEqual(len(json.loads(raw.manifest.read_text())['profiles']),1);self.assertTrue(Path(first['path']).exists())
    def test_budget_failure_precedes_probe_and_retention_charges_once(self):
        with self.assertRaises(ValueError):self.prepare(self.perf.Budget(100))
        self.assertFalse(self.calls)
        budget=self.perf.Budget(5*1024*1024);raw=self.prepare(budget);remaining=budget.remaining
        row=self.row();source,cmd=self.source(row);raw.capture(cmd,row)
        self.assertEqual(budget.remaining,remaining);self.assertLess(remaining,5*1024*1024-3*1048576)
    def test_path_substitution_and_postflight_tool_mutation_reject(self):
        raw=self.prepare()
        self.assertTrue(hasattr(raw,'preflight'),'missing pinned runtime preflight')
        with mock.patch.object(self.raw.shutil,'which',return_value='/untrusted/valgrind'),self.assertRaises(ValueError):
            raw.preflight(['valgrind'],{'PATH':'/untrusted'})
    def test_postflight_tool_mutation_reject(self):
        raw=self.prepare()
        self.tools['perl'].write_bytes(b'changed interpreter')
        with self.assertRaises(ValueError):raw.verify()
    def test_raw_workflow_literal_toggle_and_default_projection(self):
        from test_core_identity_harness import literal_run
        workflow=(ROOT/'.github/workflows/core-identity.yml').read_text()
        self.assertIn('diagnose_cortex_m_raw_profiles:',workflow)
        shell=literal_run(workflow,'Qualify three-board Cortex-M performance evidence')
        bindir=self.path/'bin';bindir.mkdir()
        for name,body in {'valgrind':'printf "valgrind-3.22.0\\n"','python':'printf "%s\\n" "$@"'}.items():
            f=bindir/name;f.write_text('#!/bin/sh\n'+body+'\n');f.chmod(0o755)
        (self.path/'perf-build-evidence').mkdir();(self.path/'perf-build-evidence/valgrind-version.txt').write_text('valgrind-3.22.0\n')
        environment={**os.environ,'PATH':str(bindir)+os.pathsep+os.environ['PATH'],'BASE_SHA':'a'*40,'HEAD_SHA':'b'*40,'IDENTITY_SUITE':'cortex-m','STEP_COUNTS':'5000000 40000000'}
        results={}
        for rawflag,common in [('false','false'),('false','true'),('true','true'),('true','false'),('wrong','true')]:
            result=subprocess.run(['bash','-c',shell],cwd=self.path,env={**environment,'DIAGNOSE_RAW_PROFILES':rawflag,'DIAGNOSE_COMMON_FIXTURES':common},capture_output=True,text=True)
            results[(rawflag,common)]=result
        self.assertEqual(results[('false','false')].returncode,0);self.assertNotIn('--diagnose-raw-l476-repeats',results[('false','false')].stdout)
        self.assertEqual(results[('true','true')].returncode,0);self.assertIn('--diagnose-raw-l476-repeats',results[('true','true')].stdout)
        self.assertEqual(results[('false','true')].returncode,0);self.assertNotIn('--diagnose-raw-l476-repeats',results[('false','true')].stdout)
        for key in [('true','false'),('wrong','true')]:self.assertNotEqual(results[key].returncode,0);self.assertNotIn('qualify',results[key].stdout)
    def test_fixed_probes_and_pinned_tool_provenance(self):
        raw=self.prepare();self.assertEqual(self.calls,[list(c) for c in self.raw.VERSION_COMMANDS])
        self.assertEqual(raw.report['tool']['annotator']['sha256'],self.raw.ANNOTATOR_SHA256)
        self.assertEqual(raw.report['tool']['package_version'],'1:3.22.0-0ubuntu3')
        self.assertEqual((raw.directory/'callgrind_annotate').stat().st_mode&0o222,0)
    def test_tool_mutation_wrong_version_and_version_signal_fail(self):
        self.tools['annotator'].write_bytes(b'wrong')
        with self.assertRaises(ValueError):self.prepare()
        self.tools['annotator'].write_bytes(b'annotator synthetic official bytes')
        import shutil
        shutil.rmtree(self.path/'output')
        for code,out in ((7,b'wrong'),(0,b'wrong'),(-15,b'wrong')):
            def fail(argv,**kwargs):
                r=subprocess.CompletedProcess(argv,code,out,b'');r.producer_exit_code=code;return r
            with mock.patch.object(self,'version_collect',side_effect=fail),self.assertRaises(ValueError):self.prepare()
            shutil.rmtree(self.path/'output')
    def test_unregistered_arbitrary_relative_traversal_and_replaced_parent_reject(self):
        raw=self.prepare();row=self.row();source,cmd=self.source(row)
        source2=self.path/'arbitrary-retained-source.callgrind';source.rename(source2)
        for path in (str(source2),'relative.callgrind',str(source.parent/'..'/'cg.out')):
            altered=list(cmd);altered[1]='--callgrind-out-file='+path;row['argv']=altered
            with self.subTest(path=path),self.assertRaises(ValueError):raw.capture(altered,row)
        source2.rename(source)
        moved=source.parent.with_name('replaced-parent');source.parent.rename(moved);source.parent.mkdir(mode=0o700)
        (source.parent/'cg.out').write_bytes((moved/'cg.out').read_bytes())
        row['argv']=cmd
        with self.assertRaises(ValueError):raw.capture(cmd,row)
    def test_header_swap_to_fifo_is_rejected_without_unprotected_reopen(self):
        raw=self.prepare();row=self.row();source,cmd=self.source(row)
        original=self.raw.identity
        def swap(path,cap):
            result=original(path,cap)
            if Path(path)==source and not source.is_fifo():source.unlink();os.mkfifo(source)
            return result
        # An unsafe Path.open must never occur, including after a safe stat/hash.
        real_open=Path.open
        def protected(path,*args,**kwargs):
            if path==source:raise AssertionError('unsafe header reopen would block on FIFO')
            return real_open(path,*args,**kwargs)
        with mock.patch.object(self.raw,'identity',side_effect=swap),mock.patch.object(Path,'open',new=protected):
            with self.assertRaises((ValueError,OSError)):raw.capture(cmd,row)

    def test_copy_descriptor_replacement_rejects_before_destination_creation(self):
        row=self.row();source,cmd=self.source(row);destination=self.path/'copy-target'
        original=self.raw.identity;created=[];real_open=Path.open
        def swap(path,cap):
            result=original(path,cap)
            if Path(path)==source and not source.is_fifo():source.unlink();os.mkfifo(source)
            return result
        def observe(path,*args,**kwargs):
            if path==destination:created.append(path)
            return real_open(path,*args,**kwargs)
        with mock.patch.object(self.raw,'identity',side_effect=swap),mock.patch.object(Path,'open',new=observe):
            with self.assertRaises(ValueError):self.raw.copy_exact(source,destination,1048576)
        self.assertEqual(created,[])
    def test_fixed_artifact_cap_includes_build_logs_and_final_receipt(self):
        raw=self.prepare();build=self.path/'perf-build-evidence';build.mkdir()
        f=build/'bounded.stdout'
        with f.open('wb') as stream:stream.truncate(8*1024*1024)
        self.assertTrue(hasattr(raw,'artifact_bytes'),'missing fixed total artifact fence')
        with self.assertRaises(ValueError):raw.artifact_bytes(1024)
        f.unlink();self.assertLess(raw.artifact_bytes(1024),8*1024*1024)
    def test_raw_file_fifo_and_corrupted_checkpoint_fail_without_unlinking_protected(self):
        raw=self.prepare();row=self.row();source,cmd=self.source(row);source.unlink();os.mkfifo(source)
        with self.assertRaises(ValueError):raw.capture(cmd,row)
        self.assertTrue(source.exists())
        raw.manifest.write_text('external owner changed manifest')
        with self.assertRaises(ValueError):raw.verify()
        self.assertEqual(raw.manifest.read_text(),'external owner changed manifest')
    def test_invalid_flag_combination_precedes_source_or_native(self):
        args=types.SimpleNamespace(diagnose_raw_l476_repeats=True,diagnose_common_fixtures=False,output=str(self.path/'out'),suite='cortex-m')
        with mock.patch.object(self.perf,'source_contract',side_effect=AssertionError('source reached')),self.assertRaisesRegex(ValueError,'requires common'):
            self.perf.qualify(args)
    def test_full_48_call_qualifier_retains_three_profiles_but_unequal_counts_fail(self):
        from test_cortex_m_perf_evidence import PerformanceControls
        fixture=PerformanceControls();fixture.path=self.path
        sources,binpath=fixture.synthetic_sources();identity=fixture.identity()
        for side,(source,_) in sources.items():
            tool=source/'scripts/perf/board_perf.py'
            text=tool.read_text().replace('import os,re,subprocess,sys,json','import os,re,subprocess,sys,json,tempfile')
            text=text.replace('def main():',"""def build_fixtures(names):
    assert names=={'nrf','stm32'}
    return {name:Path.cwd()/('perf-spin-'+name) for name in names},{}
def discover_chips():return {}
def plan_coverage(_):return {'nrf52840':'nrf','stm32f405':'stm32','stm32l476':'stm32'},{}
def fixture_origins(name):return (0 if name=='nrf' else 0x08000000,0x20000000)
def main():""")
            text=text.replace('measured={}\n',"measured={}\n    firmware,_=build_fixtures({'nrf','stm32'})\n")
            text=text.replace("'fixture.elf'","str(firmware['nrf' if board=='nrf52840' else 'stm32'])")
            text=text.replace('subprocess.run(cmd,capture_output=True,text=True,env=env)',"with tempfile.TemporaryDirectory() as temp:\n                        cmd[1:1]=['--tool=callgrind','--callgrind-out-file='+temp+'/cg.out','--cache-sim=no','--branch-sim=no']\n                        subprocess.run(cmd,capture_output=True,text=True,env=env)")
            original_node=next(n for n in ast.parse((ROOT/'scripts/perf/board_perf.py').read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='measure_once')
            original_function=ast.unparse(original_node)
            text=text.replace('from pathlib import Path','from pathlib import Path\nfrom collections import namedtuple')
            text=text.replace('def main():',"Run=namedtuple('Run','irefs instructions steps_per_batch tick_interval',defaults=(None,None,None))\nMODE_BATCH='batch'\nMODE_STEP='step'\nModeNotTakenError=ValueError\n"+original_function+'\ndef main():')
            text=text.replace("with tempfile.TemporaryDirectory() as temp:\n                        cmd[1:1]=['--tool=callgrind','--callgrind-out-file='+temp+'/cg.out','--cache-sim=no','--branch-sim=no']\n                        subprocess.run(cmd,capture_output=True,text=True,env=env)","measure_once(Path(cli),Path('configs/chips/'+board+'.yaml'),firmware['nrf' if board=='nrf52840' else 'stm32'],count,mode)")
            tool.write_text(text);(source/'Cargo.toml').write_text('[workspace]\n')
            for name,address in (('nrf',0),('stm32',0x08000000)):(source/('perf-spin-'+name)).write_bytes(elf_bytes(side.encode(),address=address))
            subprocess.run(['git','add','.'],cwd=source,check=True,capture_output=True)
            subprocess.run(['git','-c','user.name=Fixture','-c','user.email=fixture@example.invalid','-c','core.hooksPath=/dev/null','-c','commit.gpgsign=false','commit','-qm','raw synthetic fixture'],cwd=source,check=True,capture_output=True)
            sha=subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip();sources[side]=(source,sha);(identity/f'identity-{side}/sha').write_text(sha)
        args=types.SimpleNamespace(base_dir=str(sources['base'][0]),head_dir=str(sources['head'][0]),base_sha=sources['base'][1],head_sha=sources['head'][1],identity_dir=str(identity),steps='5000000 40000000',suite='cortex-m',output=str(self.path/'evidence'),diagnose_common_fixtures=True,diagnose_raw_l476_repeats=True)
        calls=[];seen={}
        def producer(cmd,**kwargs):
            if cmd[0].startswith('/usr/bin/'):return self.version_collect(cmd,**kwargs)
            self.assertEqual(cmd[0],'valgrind');calls.append(cmd)
            chip=Path(cmd[cmd.index('--chip')+1]).stem;count=int(cmd[cmd.index('--max-steps')+1]);batch='--batched' in cmd;side=Path(kwargs.get('cwd') or os.getcwd()).name
            key=(side,chip,count,batch);repeat=seen.get(key,0);seen[key]=repeat+1;ir=count*(2 if batch else 100)+1000
            if key==('base','stm32l476',200000,True):ir+=repeat
            target=Path(next(x.split('=',1)[1] for x in cmd if x.startswith('--callgrind-out-file=')))
            target.write_text('version: 1\ncreator: callgrind-3.22.0\ncmd: '+' '.join(cmd[5:])+f'\nevents: Ir\nsummary: {ir}\nfn=(1) fixture\n1 {ir}\n')
            stderr=f'==1== I refs: {ir}\n'
            if batch:stderr+=f'[batched] instructions={count} batches={count//1000} steps_per_batch=1000 tick_interval=1024\n'
            r=subprocess.CompletedProcess(cmd,0,b'',stderr.encode());r.producer_exit_code=0;return r
        before=(sys.argv[:],os.getcwd(),dict(os.environ))
        with mock.patch.dict(sys.modules,cortex_m_raw_profiles=self.raw),mock.patch.object(self.perf,'collect',side_effect=producer):
            with self.assertRaisesRegex(ValueError,'UNSTABLE'):self.perf.qualify(args)
        evidence=json.loads((self.path/'evidence/qualification.json').read_text())
        self.assertEqual(len(calls),48);self.assertEqual(len(evidence['raw_invocations']),48);self.assertFalse(evidence['qualified'])
        self.assertTrue(evidence['raw_profile_diagnosis']['complete']);self.assertEqual(len(evidence['raw_profile_diagnosis']['profiles']),3)
        self.assertTrue(all('callgrind_annotation' not in row for row in evidence['raw_invocations']))
        self.assertEqual((sys.argv,os.getcwd(),dict(os.environ)),before)
        self.assertLess(sum(x.stat().st_size for x in (self.path/'evidence').rglob('*') if x.is_file()),8*1024*1024)

    def test_actual_original_measure_once_copies_before_temporary_cleanup(self):
        from collections import namedtuple
        common_path=self.path/'stm32.elf';common_path.write_bytes(elf_bytes())
        common=types.SimpleNamespace(paths={'stm32':common_path.resolve()},verify=lambda p:dict(path=str(p),bytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest()))
        raw=self.prepare();rows=[];module=types.SimpleNamespace(IREFS_RE=re.compile(r'^==\d+== I refs: ([\d,]+)',re.M),BATCHED_RE=re.compile(r'^\[batched\] instructions=(\d+) batches=(\d+) steps_per_batch=([\d.]+) tick_interval=(\d+)',re.M))
        runner=self.perf.BoundedRunner(module,'base',self.path/'output',rows,self.perf.Budget(5*1024*1024),common,raw_profiles=raw);paths=[]
        def producer(cmd,**kwargs):
            target=Path(next(x.split('=',1)[1] for x in cmd if x.startswith('--callgrind-out-file=')));paths.append(target);ir=10+len(paths)
            target.write_text('version: 1\ncreator: callgrind-3.22.0\ncmd: '+' '.join(cmd[5:])+f'\nevents: Ir\nsummary: {ir}\nfn=(1) fixture\n1 {ir}\n')
            r=subprocess.CompletedProcess(cmd,0,b'',f'==1== I refs: {ir}\n[batched] instructions=200000 batches=196 steps_per_batch=1020.41 tick_interval=1024\n'.encode());r.producer_exit_code=0;return r
        cli=self.path/'cli';cli.write_bytes(b'fake cli')
        node=next(n for n in ast.parse((ROOT/'scripts/perf/board_perf.py').read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='measure_once')
        ns=dict(Path=Path,Run=namedtuple('Run','irefs instructions steps_per_batch tick_interval',defaults=(None,None,None)),os=os,tempfile=tempfile,subprocess=runner,MODE_BATCH='batch',MODE_STEP='step',IREFS_RE=module.IREFS_RE,BATCHED_RE=module.BATCHED_RE,ModeNotTakenError=ValueError)
        exec(compile(ast.Module(body=[node],type_ignores=[]),'original-measure-once','exec'),ns)
        module.measure_once=ns['measure_once']
        with mock.patch.object(self.perf,'collect',side_effect=producer):
            for _ in range(3):ns['measure_once'](cli,self.path/'stm32l476.yaml',common_path,200000,'batch')
        self.assertTrue(all(not p.exists() for p in paths));self.assertEqual(len(rows),3);self.assertTrue(raw.finish()['complete'])

if __name__=='__main__':unittest.main()
