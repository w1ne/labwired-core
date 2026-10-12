"""Bounded controls for the optional performance lane; no Cargo or emulator."""

import copy
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

from test_core_identity_harness import literal_run
from test_cortex_m_oracle_evidence import fixture_bytes

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts/ci/cortex_m_perf_evidence.py"
BOARDS = ("nrf52840", "stm32f405", "stm32l476")


def records():
    result = []
    for side, board, mode, steps in itertools.product(
        ("base", "head"), BOARDS, ("step", "batch"), (200000, 1200000)
    ):
        for repeat in range(3 if mode == "batch" else 1):
            result.append(dict(side=side, board=board, mode=mode, steps=steps,
                repeat=repeat, exit_code=0, producer_exit_code=0,
                argv=['valgrind',f'/fixture/{side}/target/release/labwired','run','--chip',f'configs/chips/{board}.yaml','--firmware','fixture.elf','--max-steps',str(steps)] + (['--batched'] if mode=='batch' else []),
                cwd=f'/fixture/{side}', selected_flags=['--batched'] if mode=='batch' else [],
                mode_environment={'LABWIRED_ARM_SINGLE_STEP':{'present':mode=='step','value':'1' if mode=='step' else None}},
                ir=steps * (100 if mode == "step" else 2) + 1000,
                instructions=steps if mode == "batch" else None,
                batches=steps // 1000 if mode == "batch" else None,
                steps_per_batch=1000.0 if mode == "batch" else None,
                tick_interval=1024 if mode == "batch" else None,
                fixture_sha256="a" * 64, cli_sha256=("b" if side == "base" else "c") * 64))
    return result


class PerformanceControls(unittest.TestCase):
    def setUp(self):
        self.assertTrue(HELPER.is_file(), "optional performance helper missing")
        spec = importlib.util.spec_from_file_location("perf_evidence", HELPER)
        self.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.helper)
        self.temp = tempfile.TemporaryDirectory(prefix="cortex-perf-control-")
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)

    def test_exact_sha_rejects_refs_and_partial_hashes(self):
        self.helper.validate_sha("a" * 40)
        for value in ("main", "a" * 39, "A" * 40, "a" * 41, "a" * 40 + "\n"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.helper.validate_sha(value)

    def test_complete_48_calls_and_step_observation_limit(self):
        report = self.helper.validate_records(records(), 1024)
        self.assertEqual(report["invocations"], 48)
        self.assertEqual(len(report["measurements"]), 12)
        self.assertFalse(report["step_repeated_stability_proven"])

    def test_missing_duplicate_extra_board_mode_or_repeat_fails(self):
        for change in (lambda rows: rows.pop(), lambda rows: rows.append(rows[0]),
                       lambda rows: rows[0].update(board="stm32f103"),
                       lambda rows: rows[0].update(mode="other"),
                       lambda rows: rows[0].update(repeat=1)):
            rows = records(); change(rows)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.helper.validate_records(rows, 1024)

    def test_positive_count_with_failed_child_is_rejected(self):
        rows = records(); rows[0]["exit_code"] = 1
        with self.assertRaises(ValueError):
            self.helper.validate_records(rows, 1024)

    def test_bad_ir_retired_count_tick_and_fixture_are_rejected(self):
        for field, value in (("ir", 0), ("ir", -1), ("instructions", 1),
                             ("tick_interval", 1), ("batches", 0),
                             ("steps_per_batch", float("nan")), ("fixture_sha256", "bad")):
            rows = records(); next(row for row in rows if row["mode"] == "batch")[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.helper.validate_records(rows, 1024)

    def test_raw_invocation_tampering_or_oversized_arguments_fail(self):
        mutations = [lambda row: row.update(producer_exit_code=7),
            lambda row: row.update(cwd='relative'),
            lambda row: row.update(mode_environment={'LABWIRED_ARM_SINGLE_STEP':{'present':False,'value':None}}),
            lambda row: row.update(selected_flags=['--batched']),
            lambda row: row['argv'].__setitem__(row['argv'].index('--max-steps')+1,'1'),
            lambda row: row['argv'].__setitem__(row['argv'].index('--chip')+1,'other.yaml'),
            lambda row: row['argv'].append('x'*2049)]
        for mutation in mutations:
            rows = records(); mutation(rows[0])
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.helper.validate_records(rows,1024)
        with self.assertRaises(ValueError):
            self.helper.bounded_argv(['valgrind'] + ['x'*1000]*9)

    def test_batch_instability_or_changed_fixture_fails(self):
        for field, value in (("ir", 1234567), ("fixture_sha256", "d" * 64)):
            rows = records(); next(row for row in rows if row["mode"] == "batch" and row["repeat"] == 2)[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.helper.validate_records(rows, 1024)

    def test_negative_slope_fails(self):
        rows = records()
        for row in rows:
            if row["steps"] == 1200000:
                row["ir"] = 1
        with self.assertRaises(ValueError):
            self.helper.validate_records(rows, 1024)

    def identity(self):
        for side, passes in (("base", (1, 2)), ("head", (1,))):
            folder = self.path / f"identity-{side}"; folder.mkdir()
            (folder / "sha").write_text(("a" if side == "base" else "b") * 40)
            for board, mode, count, repeat in itertools.product(BOARDS, ("step", "batched"), ("5000000", "40000000"), passes):
                prefix = folder / f"cortex-m-{board}-{count}-{mode}-p{repeat}"
                Path(str(prefix) + ".exit").write_text("0\n")
                Path(str(prefix) + ".stdout").write_text("TIER1 pass\n")
                Path(str(prefix) + ".stderr").write_text("")
                events = [dict(seq=i, cycle=i + 1, bus="uart2", payload=dict(protocol="uart", direction="tx", byte=c)) for i, c in enumerate(b"TIER1 pass\n")]
                Path(str(prefix) + ".bus.json").write_text(json.dumps(events))
        return self.path

    def test_complete_identity_positive_cycles_and_hashes(self):
        proof = self.helper.validate_identity(self.identity(), "5000000 40000000", "a" * 40, "b" * 40)
        self.assertEqual(proof["positive_execution_cells"], 36)
        self.assertEqual(proof["stable_identical_streams"], 48)

    def test_mixed_zero_startup_and_positive_uart_cycles_are_valid(self):
        self.identity()
        for path in self.path.glob('identity-*/*.bus.json'):
            events = json.loads(path.read_text())
            events.insert(0, dict(seq=0, cycle=0, bus='gpioa', payload=dict(pin=0, level=0)))
            path.write_text(json.dumps(events))
        proof = self.helper.validate_identity(self.path, '5000000 40000000', 'a'*40, 'b'*40)
        self.assertEqual(proof['positive_execution_cells'], 36)

    def test_zero_uart_positive_non_uart_negative_noninteger_cycles_fail(self):
        self.identity()
        path = next((self.path/'identity-head').glob('*.bus.json'))
        original = json.loads(path.read_text())
        variants = []
        zero = copy.deepcopy(original)
        for event in zero:
            event['cycle'] = 0
        variants.append(zero)
        variants.append(zero + [dict(seq=100,cycle=1,bus='gpioa',payload={})])
        for cycle in (-1, 1.5, True, '1'):
            events = copy.deepcopy(original); events[0]['cycle'] = cycle; variants.append(events)
        for events in variants:
            path.write_text(json.dumps(events))
            with self.subTest(events=events), self.assertRaises(ValueError):
                self.helper.validate_identity(self.path, '5000000 40000000', 'a'*40, 'b'*40)

    def test_missing_identity_cell_invalid_json_no_cycles_or_uart_fails(self):
        self.identity()
        file = next((self.path / "identity-head").glob("*.bus.json")); original = file.read_bytes()
        for value in (b"", b"{}", b"[]", b"not json", original.replace(b'"cycle": 1', b'"cycle": -1')):
            file.write_bytes(value)
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.helper.validate_identity(self.path, "5000000 40000000", "a" * 40, "b" * 40)
        file.write_bytes(original); file.unlink()
        with self.assertRaises(ValueError):
            self.helper.validate_identity(self.path, "5000000 40000000", "a" * 40, "b" * 40)

    def test_streamed_dual_pipe_overflow_and_timeout_are_bounded(self):
        for script, code, kwargs in (("import os;os.write(1,b'x'*10001);os.write(2,b'y'*10001)", 125, {"stream_limit": 10000}),
                                     ("import time;time.sleep(5)", 124, {"timeout": 0.1})):
            run = self.helper.collect([sys.executable, "-c", script], **kwargs)
            self.assertEqual(run.returncode, code)
            self.assertLessEqual(len(run.stdout), kwargs.get("stream_limit", 262144))
            self.assertLessEqual(len(run.stderr), kwargs.get("stream_limit", 262144))

    def test_failed_and_signal_status_are_not_lost(self):
        for script, status in (("import sys;sys.exit(7)", 7), ("import os,signal;os.kill(os.getpid(),signal.SIGTERM)", -15)):
            run = self.helper.collect([sys.executable, "-c", script])
            self.assertEqual(run.returncode, status)

    def test_owned_child_is_stopped_on_timeout(self):
        pidfile = self.path / "child.pid"
        script = f"import subprocess,time,pathlib; p=subprocess.Popen(['{sys.executable}','-c','import time;time.sleep(10)']);pathlib.Path({str(pidfile)!r}).write_text(str(p.pid));time.sleep(10)"
        run = self.helper.collect([sys.executable, "-c", script], timeout=0.2)
        self.assertEqual(run.returncode, 124)
        pid = int(pidfile.read_text())
        try:
            state = Path(f"/proc/{pid}/stat").read_text().split()[2]
            self.assertEqual(state, "Z")
        except FileNotFoundError:
            pass

    def test_wrapper_restores_module_argv_cwd_and_environment_on_error(self):
        module = types.SimpleNamespace(subprocess=subprocess, main=lambda: (_ for _ in ()).throw(ValueError("failure")))
        before = (subprocess, sys.argv[:], os.getcwd(), dict(os.environ))
        with self.assertRaises(ValueError):
            self.helper.invoke_tool(module, self.path, self.path / "cli", "base", self.path, [], self.helper.Budget())
        self.assertIs(module.subprocess, before[0]); self.assertEqual(sys.argv, before[1])
        self.assertEqual(os.getcwd(), before[2]); self.assertEqual(dict(os.environ), before[3])

    def test_status_completeness_stale_is_preserved_and_regression_fails(self):
        measures = self.helper.validate_records(records(), 1024)["measurements"]
        status = dict(ok=False, measured={b:dict(step=100.0,batch=2.0) for b in BOARDS}, skipped={}, unmeasurable=[], contract_failures=[], regressions=[], stale=[dict(board="nrf52840",mode="step")])
        result = self.helper.validate_status(status, "base", measures, 1)
        self.assertFalse(result["committed_baseline_pass"])
        for mutation in (lambda s:s["measured"].pop("nrf52840"), lambda s:s["skipped"].update(nrf52840="no fixture"), lambda s:s["regressions"].append({}), lambda s:s["measured"]["nrf52840"].update(step=999)):
            changed = copy.deepcopy(status); mutation(changed)
            with self.assertRaises(ValueError):
                self.helper.validate_status(changed, "base", measures, 1)

    def test_default_false_and_actual_job_shell_rejects_mutable_refs(self):
        workflow = (ROOT / ".github/workflows/core-identity.yml").read_text()
        self.assertRegex(workflow, r"qualify_cortex_m_perf:\n(?:.*\n){1,3}\s+default: false")
        shell = literal_run(workflow, "Fence exact Cortex-M performance sources")
        env = dict(os.environ, BASE_SHA="main", HEAD_SHA="b" * 40, IDENTITY_SUITE="cortex-m", STEP_COUNTS="5000000 40000000")
        run = subprocess.run(["bash", "-c", shell], cwd=self.path, env=env, capture_output=True)
        self.assertNotEqual(run.returncode, 0)

    def test_original_jobs_and_five_input_defaults_are_preserved(self):
        workflow = (ROOT / ".github/workflows/core-identity.yml").read_text()
        hashes = {
            "cortex-m-oracles": "104bdba78449c9a4db0fcd964b1a68986e054a145abbf786383478aef3975cf7",
            "run": "734d2297b5d540d5f1fd155d657112c28688de4a382f1d42ae2b330e9e2f74f8",
            "compare": "0f381dabd5400a976c8e2dc1a4201b32d30e9bbaa76428b0a80c6695d16c8971",
        }
        import re
        for name, expected in hashes.items():
            start = workflow.index("\n  " + name + ":\n")
            next_job = re.search(r"\n  [a-z][a-z-]+:\n", workflow[start + 1:])
            end = start + 1 + next_job.start() if next_job else len(workflow)
            self.assertEqual(hashlib.sha256(workflow[start:end].encode()).hexdigest(), expected, name)
        self.assertIn('default: "main"', workflow)
        self.assertIn('default: "5000000 40000000"', workflow)
        self.assertIn('options: [esp32s3, cortex-m, all]\n        default: esp32s3', workflow)
        self.assertIn('permissions:\n  contents: read\n', workflow)
        self.assertNotIn('issues: write', workflow)

    def test_literal_diagnostic_toggle_preserves_default_argv_and_rejects_invalid(self):
        workflow=(ROOT/'.github/workflows/core-identity.yml').read_text()
        self.assertRegex(workflow,r'diagnose_cortex_m_common_fixtures:\n(?:.*\n){1,3}\s+default: false')
        self.assertIn('DIAGNOSE_COMMON_FIXTURES: ${{ inputs.diagnose_cortex_m_common_fixtures }}',workflow)
        shell=literal_run(workflow,'Qualify three-board Cortex-M performance evidence')
        binary=self.path/'bin';binary.mkdir()
        for name,body in (('valgrind','#!/bin/sh\nprintf "synthetic version\\n"\n'),
                          ('python','#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\nPath(os.environ["CAPTURE_ARGV"]).write_text(json.dumps(sys.argv[1:]))\n')):
            target=binary/name;target.write_text(body);target.chmod(0o755)
        folder=self.path/'perf-build-evidence';folder.mkdir()
        (folder/'valgrind-version.txt').write_text('synthetic version\n')
        expected=['scripts/ci/cortex_m_perf_evidence.py','qualify','--base-dir','perf-base','--head-dir','perf-head',
                  '--base-sha','a'*40,'--head-sha','b'*40,'--identity-dir','perf-identity','--steps',
                  '5000000 40000000','--suite','cortex-m','--output','perf-evidence']
        for toggle in ('false','true','invalid'):
            capture=self.path/('argv-'+toggle+'.json')
            env=dict(os.environ,PATH=str(binary)+os.pathsep+os.environ['PATH'],BASE_SHA='a'*40,HEAD_SHA='b'*40,
                     IDENTITY_SUITE='cortex-m',STEP_COUNTS='5000000 40000000',DIAGNOSE_COMMON_FIXTURES=toggle,CAPTURE_ARGV=str(capture))
            result=subprocess.run(['bash','-c',shell],cwd=self.path,env=env,capture_output=True)
            if toggle=='invalid':
                self.assertNotEqual(result.returncode,0);self.assertFalse(capture.exists())
            else:
                self.assertEqual(result.returncode,0,result.stderr.decode())
                wanted=expected[:2]+(['--diagnose-common-fixtures'] if toggle=='true' else [])+expected[2:]
                self.assertEqual(json.loads(capture.read_text()),wanted)

    def test_literal_build_route_uses_both_exact_sources_and_required_feature(self):
        sources, binpath = self.synthetic_sources()
        (binpath / 'python').symlink_to(sys.executable)
        for side, (path, _) in sources.items():
            path.rename(self.path / ('perf-' + side))
        helper_path = self.path / 'scripts/ci/cortex_m_perf_evidence.py'
        helper_path.parent.mkdir(parents=True)
        helper_path.write_bytes(HELPER.read_bytes())
        (self.path/'perf-build-evidence').mkdir()
        cargo = binpath / 'cargo'
        cargo.write_text('#!/usr/bin/env python3\nimport sys\nassert sys.argv[1:]==["build","--locked","--release","-p","labwired-cli","--features","event-scheduler"]\nprint("feature-route-confirmed")\n')
        cargo.chmod(0o755)
        env = dict(os.environ, GITHUB_WORKSPACE=str(self.path), BASE_SHA=sources['base'][1], HEAD_SHA=sources['head'][1], PATH=str(binpath)+os.pathsep+os.environ['PATH'])
        for key in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_TARGET_DIR'):
            env.pop(key, None)
        shell = literal_run((ROOT/'.github/workflows/core-identity.yml').read_text(), 'Build exact Cortex-M performance CLIs')
        run = subprocess.run(['bash','-c',shell],cwd=self.path,env=env,capture_output=True)
        self.assertEqual(run.returncode,0,run.stderr.decode())
        for side in ('base','head'):
            self.assertIn('feature-route-confirmed',(self.path/f'perf-build-evidence/{side}-build.stdout').read_text())

    def test_shared_output_budget_cannot_be_exceeded(self):
        budget = self.helper.Budget(100)
        result = self.helper.collect([sys.executable, "-c", "import os;os.write(1,b'x'*80);os.write(2,b'y'*80)"], budget=budget)
        self.assertEqual(result.returncode, 125)
        self.assertEqual(len(result.stdout) + len(result.stderr), 100)
        self.assertEqual(budget.remaining, 0)

    def test_wrapper_rejects_real_failed_valgrind_with_positive_count(self):
        executable = self.path / "valgrind"
        executable.write_text("#!/bin/sh\nprintf '==1== I refs: 1000000\\n' >&2\nexit 7\n")
        executable.chmod(0o755)
        runner = self.helper.BoundedRunner(types.SimpleNamespace(), "base", self.path, [], self.helper.Budget())
        with mock.patch.dict(os.environ, PATH=str(self.path) + os.pathsep + os.environ["PATH"]):
            with self.assertRaisesRegex(ValueError, "exit=7"):
                runner.run(["valgrind"], capture_output=True, text=True)
        self.assertEqual(json.loads((self.path / "base-child-01.json").read_text())["exit_code"], 7)

    def synthetic_sources(self):
        """Execute the real optional wrapper/collector against a synthetic tool."""
        tool = '''import os,re,subprocess,sys,json
from pathlib import Path
IREFS_RE=re.compile(r"^==\\d+== I refs: ([\\d,]+)",re.M)
BATCHED_RE=re.compile(r"^\\[batched\\] instructions=(\\d+) batches=(\\d+) steps_per_batch=([\\d.]+) tick_interval=(\\d+)",re.M)
MIN_RECOMMENDED_TICK_INTERVAL=1024
def is_regression(measured,baseline,mode):return measured>baseline*1.05
def main():
    cli=sys.argv[sys.argv.index('--cli')+1]
    measured={}
    for board in ('nrf52840','stm32f405','stm32l476'):
        measured[board]={'step':100.0,'batch':2.0}
        for mode in ('step','batch'):
            for repeat in range(3 if mode=='batch' else 1):
                for count in (200000,1200000):
                    env=dict(os.environ)
                    if mode=='step':env['LABWIRED_ARM_SINGLE_STEP']='1'
                    cmd=['valgrind',cli,'run','--chip','configs/chips/'+board+'.yaml','--firmware','fixture.elf','--max-steps',str(count)]
                    if mode=='batch':cmd+=['--batched']
                    subprocess.run(cmd,capture_output=True,text=True,env=env)
    Path(sys.argv[sys.argv.index('--status-json')+1]).write_text(json.dumps(dict(ok=True,measured=measured,skipped={},unmeasurable=[],contract_failures=[],regressions=[],stale=[])))
    return 0
'''
        sources = {}
        for side in ("base", "head"):
            path = self.path / side; path.mkdir()
            for name, content in {"Cargo.lock":"lock", ".cargo/config.toml":"config", "scripts/perf/board_perf.py":tool,
                                  "scripts/perf/baselines.json":"{}", "crates/firmware-perf-spin/src/main.rs":"fixture"}.items():
                file = path / name; file.parent.mkdir(parents=True, exist_ok=True); file.write_text(content)
            for board in BOARDS:
                file = path / f"configs/chips/{board}.yaml"; file.parent.mkdir(parents=True, exist_ok=True); file.write_text(board)
            (path / "fixture.elf").write_bytes(fixture_bytes())
            (path / "side-marker").write_text(side)
            for args in (("init", "-q"), ("add", "."), ("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "synthetic")):
                subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *args], cwd=path, check=True, capture_output=True)
            sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path, text=True).strip()
            cli = path / "target/release/labwired"; cli.parent.mkdir(parents=True); cli.write_text("#!/bin/sh\nexit 0\n"); cli.chmod(0o755)
            sources[side] = (path, sha)
        binpath = self.path / "bin"; binpath.mkdir()
        executable = binpath / "valgrind"
        executable.write_text('''#!/usr/bin/env python3
import sys
count=int(sys.argv[sys.argv.index('--max-steps')+1]);batch='--batched' in sys.argv
print('==1== I refs: '+str(count*(2 if batch else 100)+1000),file=sys.stderr)
if batch:print('[batched] instructions='+str(count)+' batches='+str(count//1000)+' steps_per_batch=1000 tick_interval=1024',file=sys.stderr)
''')
        executable.chmod(0o755)
        return sources, binpath

    def test_actual_helper_runs_48_calls_against_synthetic_sources(self):
        sources, binpath = self.synthetic_sources()
        identity = self.identity()
        for side, (_, sha) in sources.items():
            (identity / f"identity-{side}/sha").write_text(sha)
        args = types.SimpleNamespace(base_dir=str(sources['base'][0]),head_dir=str(sources['head'][0]),
            base_sha=sources['base'][1],head_sha=sources['head'][1],identity_dir=str(identity),steps='5000000 40000000',suite='cortex-m',output=str(self.path/'evidence'))
        with mock.patch.dict(os.environ, PATH=str(binpath)+os.pathsep+os.environ['PATH'], LABWIRED_ARM_SINGLE_STEP="1", UNRELATED_FIXTURE_VALUE='must-stay-out-of-receipt'):
            self.assertEqual(self.helper.qualify(args), 0)
            self.assertEqual(os.environ['LABWIRED_ARM_SINGLE_STEP'], "1")
        evidence = json.loads((self.path/'evidence/qualification.json').read_text())
        self.assertNotIn('must-stay-out-of-receipt',(self.path/'evidence/qualification.json').read_text())
        self.assertEqual(len(evidence['raw_invocations']),48)
        self.assertTrue(evidence['qualified'])
        self.assertFalse(evidence['step_repeated_stability_proven'])
        for row in evidence['raw_invocations']:
            self.assertEqual(row['argv'][0],'valgrind')
            self.assertEqual(row['cwd'],str(sources[row['side']][0].resolve()))
            self.assertEqual(row['producer_exit_code'],0)
            expected = {'present':True,'value':'1'} if row['mode']=='step' else {'present':False,'value':None}
            self.assertEqual(row['mode_environment'],{'LABWIRED_ARM_SINGLE_STEP':expected})
            self.assertEqual(row['selected_flags'],['--batched'] if row['mode']=='batch' else [])
            self.assertEqual(row['argv'][row['argv'].index('--max-steps')+1],str(row['steps']))

    def test_changed_contract_or_wrong_checkout_fails_before_measurement(self):
        sources, _ = self.synthetic_sources()
        path, sha = sources['base']
        with self.assertRaises(ValueError):
            self.helper.source_contract(path, 'a'*40)
        before = self.helper.source_contract(path, sha)
        (path/'Cargo.lock').write_text('different')
        self.assertNotEqual(self.helper.source_contract(path, sha), before)

    def test_source_change_regression_fails_with_comparison_receipt(self):
        sources, binpath = self.synthetic_sources()
        for path, _ in sources.values():
            tool = path / 'scripts/perf/board_perf.py'
            tool.write_text(tool.read_text().replace("'step':100.0", "'step':120.0 if Path.cwd().name=='head' else 100.0"))
        executable = binpath / 'valgrind'
        executable.write_text(executable.read_text().replace('(2 if batch else 100)', "(2 if batch else 120 if '/head/' in sys.argv[1] else 100)"))
        self.identity()
        for side, (_, sha) in sources.items():
            (self.path / f'identity-{side}/sha').write_text(sha)
        args = types.SimpleNamespace(base_dir=str(sources['base'][0]),head_dir=str(sources['head'][0]),
            base_sha=sources['base'][1],head_sha=sources['head'][1],identity_dir=str(self.path),steps='5000000 40000000',suite='cortex-m',output=str(self.path/'evidence'))
        with mock.patch.dict(os.environ, PATH=str(binpath)+os.pathsep+os.environ['PATH']):
            with self.assertRaisesRegex(ValueError,'source-change A/B regression'):
                self.helper.qualify(args)
        evidence = json.loads((self.path/'evidence/qualification.json').read_text())
        self.assertFalse(evidence['qualified'])
        self.assertEqual(len(evidence['raw_invocations']),48)
        self.assertEqual(sum(row['regression'] for row in evidence['comparisons']),3)

    def test_contract_mismatch_receipt_has_no_measurements(self):
        sources, _ = self.synthetic_sources()
        (sources['head'][0]/'Cargo.lock').write_text('changed')
        args = types.SimpleNamespace(base_dir=str(sources['base'][0]),head_dir=str(sources['head'][0]),
            base_sha=sources['base'][1],head_sha=sources['head'][1],identity_dir=str(self.path),steps='5000000 40000000',suite='cortex-m',output=str(self.path/'evidence'))
        with self.assertRaisesRegex(ValueError,'contract bytes differ'):
            self.helper.qualify(args)
        evidence = json.loads((self.path/'evidence/qualification.json').read_text())
        self.assertFalse(evidence['qualified'])
        self.assertEqual(evidence['raw_invocations'],[])


def load_tests(loader, tests, pattern):
    # This file is named by the existing hosted lane; no new workflow/default.
    from test_cortex_m_perf_diagnostics import DiagnosticsControls
    tests.addTests(loader.loadTestsFromTestCase(DiagnosticsControls))
    return tests


if __name__ == "__main__":
    unittest.main()
