from pathlib import Path
import subprocess,json,hashlib,time,os,signal
root=Path('/data/work-evidence/analog-initial-supernode-20261112')
deps=Path('/data/build-targets/controlled-buck-20261112/shared-runtime-integration-20261112/deps')
paths={k:deps/v for k,v in {'labwired_core':'liblabwired_core-1e12c06a019b1626.rlib','labwired_config':'liblabwired_config-b11f1e1b5c5a7cec.rlib','serde_json':'libserde_json-86add3b10d7fd404.rlib'}.items()}
args=['rustc','--edition=2021','-C','opt-level=1','-C','debuginfo=0','-L','dependency='+str(deps),str(root/'analog_construct_probe_stimulus.rs'),'-o',str(root/'analog-construct-probe-stimulus')]
for name,path in paths.items():args+=['--extern',name+'='+str(path)]
p=subprocess.run(args,capture_output=True,text=True,timeout=15)
(root/'probe-build-stimulus.log').write_text(p.stdout+p.stderr)
(root/'probe-build-stimulus.exit').write_text(str(p.returncode)+'\n')
if p.returncode:raise SystemExit(p.returncode)
receipt={'scope':'public production API constructor and one-step probe with declared defaults and explicit all-zero/disabled controller stimuli, no MCU execution','source_sha256':hashlib.sha256((root/'analog_construct_probe_stimulus.rs').read_bytes()).hexdigest(),'binary_sha256':hashlib.sha256((root/'analog-construct-probe-stimulus').read_bytes()).hexdigest(),'libraries':{k:{'path':str(v),'sha256':hashlib.sha256(v.read_bytes()).hexdigest()} for k,v in paths.items()}}
(root/'probe-build-receipt-stimulus.json').write_text(json.dumps(receipt,indent=2)+'\n')
cases=[('fc-state-r4','/data/work-scratch/native-fc-opamp-direct-cold-state-r4-20261112/model.json',True),('five-state737-r4','/data/work-scratch/native-five-mcu-opamp-direct-state-737-r4-20261112/model.json',True)]
for name,path,step in cases:
 out=root/('stimulus-'+name+'-constructor.json');err=root/('stimulus-'+name+'-constructor.log');start=time.monotonic()
 with out.open('w') as stdout,err.open('w') as stderr:
  cmd=['/usr/bin/time','-v',str(root/'analog-construct-probe-stimulus'),path]+(['one-step'] if step else [])
  p=subprocess.Popen(cmd,stdout=stdout,stderr=stderr,start_new_session=True)
  timeout=False
  try:p.wait(timeout=30)
  except subprocess.TimeoutExpired:
   timeout=True;os.killpg(p.pid,signal.SIGTERM)
   try:p.wait(timeout=2)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
 (root/('stimulus-'+name+'-constructor-receipt.json')).write_text(json.dumps({'input':path,'input_sha256':hashlib.sha256(Path(path).read_bytes()).hexdigest(),'seconds':time.monotonic()-start,'exit':p.returncode,'timeout':timeout,'stdout':str(out),'stderr':str(err)},indent=2)+'\n')
(root/'constructor-probes-stimulus.done').write_text('done\n')
