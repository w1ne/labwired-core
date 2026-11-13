
import datetime, hashlib, json, os, pathlib, shutil, subprocess, sys, time
repo=pathlib.Path('/data/worktrees/labwired-core-shared-runtime-integration-20261112')
base=pathlib.Path('/data/work-evidence/analog-initial-supernode-20261112')
out=pathlib.Path('/data/work-evidence/analog-initial-final-guard-20261112')
env=dict(os.environ)
env['PATH']='/data/work-evidence/shared-runtime-integration-20261112/cargo-bin:'+env['PATH']
env['CARGO_TARGET_DIR']='/data/build-targets/controlled-buck-20261112'
env['CARGO_BUILD_JOBS']='2'
env['CARGO_INCREMENTAL']='0'
def run(name,args,timeout=1800):
    start=time.monotonic()
    with (out/(name+'.log')).open('w') as log:
        result=subprocess.run(args,cwd=repo,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=timeout)
    (out/(name+'.exit')).write_text(str(result.returncode)+'\n')
    (out/(name+'.duration')).write_text(str(time.monotonic()-start)+'\n')
    if result.returncode: raise RuntimeError(name+' failed: '+str(result.returncode))
def capture(*args):
    return subprocess.check_output(['git','-c','core.fsmonitor=false',*args],cwd=repo,text=True).strip()
try:
    assert shutil.disk_usage('/data').free >= 5*1024**3, 'disk guard'
    assert capture('rev-parse','HEAD')=='a6faa23fd37c1be3a44110850138d5ba0233383a'
    assert not capture('status','--porcelain')
    run('cherry-pick',['git','-c','core.fsmonitor=false','cherry-pick','f2d017c70941c28c8fe818aed510ed43851cd630'],900)
    head=capture('rev-parse','HEAD')
    (out/'integrated-head.txt').write_text(head+'\n')
    tests=sorted({p.stem for p in (repo/'crates/core/tests').glob('analog_opamp*.rs')}|{'analog_transient_opamp','analog_transient_voltage_group','analog_voltage_switch_boundaries'})
    for name in tests: assert (repo/'crates/core/tests'/(name+'.rs')).is_file(), name
    receipt={'head':head,'parent':'a6faa23fd37c1be3a44110850138d5ba0233383a','cherry_pick':'f2d017c70941c28c8fe818aed510ed43851cd630','tests':tests,'profile':'shared-runtime-integration-20261112','jobs':2,'incremental':False,'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (out/'source.json').write_text(json.dumps(receipt,indent=2)+'\n')
    args=['cargo','test','-p','labwired-core']
    for name in tests: args+=['--test',name]
    run('focused',args,2400)
    # Keep every earlier result immutable. The prior owned runner is copied with
    # only its task output root redirected; the physical fixture paths stay fixed.
    original=(base/'run-constructor-probes-stimulus.py').read_text()
    copied=original.replace(str(base),str(out))
    for name in ['analog_construct_probe_stimulus.rs','analog_construct_probe.rs']:
        if (base/name).is_file(): shutil.copy2(base/name,out/name)
    (out/'run-constructor-probes-stimulus.py').write_text(copied)
    (out/'runner-provenance.json').write_text(json.dumps({'original_sha256':hashlib.sha256(original.encode()).hexdigest(),'copied_sha256':hashlib.sha256(copied.encode()).hexdigest(),'change':'only evidence-root path replacement','original_root':str(base),'new_root':str(out)},indent=2)+'\n')
    run('native-probes',['python3',str(out/'run-constructor-probes-stimulus.py')],180)
    (out/'complete.exit').write_text('0\n')
except BaseException as error:
    (out/'failure.txt').write_text(repr(error)+'\n')
    (out/'complete.exit').write_text('1\n')
    raise
