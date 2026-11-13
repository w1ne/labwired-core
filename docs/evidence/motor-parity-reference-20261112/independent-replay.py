"""Independent scalar winding/shaft reference, driven only by captured MMIO.

Contracts: RM0351 Rev10 table199, PWM1 CNT<CCR; bldc::step documented
floating-neutral/exact freewheel decay; shaft::step semi-implicit Euler.
No motor snapshot is used as an integration input. Fine runs subdivide each
CPU-cycle interval but keep the MMIO command schedule identical.
"""
import json, math, pathlib, hashlib, time
ROOT=pathlib.Path('/data/work-evidence/motor-parity-review-594a9931-20261112')
trace=json.loads((ROOT/'current-wasm-trace.json').read_text())
R=0.35; L=0.00018; KE=KT=0.04; J=1e-7; B=3e-6; V=24.0; POLES=7
HZ=80_000_000; DT=1/HZ; TAU=2*math.pi
OBS=[100000,150000,200000,250000,300000]
commands={r['cycle']:r for r in trace['register_changes']}
for row in trace['register_changes']:
 if row['cycle']>=85:
  assert (row['psc'],row['arr'],row['ccmr1'],row['ccmr2'],row['bdtr'],row['cr1'])==(3,999,26728,104,32816,1)
  assert all(((row['ccer']>>(4*k))&15) in (0,1,4) for k in range(3))
  assert (row['cnt']*4+row['psc_cnt'])==(row['cycle']-84)%4000

def shape(a):
 h=(a%TAU)/math.pi
 s=6*h if h<1/6 else 1. if h<5/6 else 6-6*h if h<7/6 else -1. if h<11/6 else 6*h-12
 if abs(s)<=8*2.220446049250313e-16:return 0.
 if abs(abs(s)-1)<=8*2.220446049250313e-16:return math.copysign(1.,s)
 return s

def project(currents,opened):
 n=2 if opened else 3
 mean=sum(currents[1:] if opened else currents)/n
 new=[0. if opened and k==0 else currents[k]-mean for k in range(3)]
 new[2]-=sum(new)
 return new

def run(subdivisions=1,end=300000,legacy=False):
 currents=[0.,0.,0.]; theta=omega=torque=bus=0.; row=commands[1]
 snapshots=[]; first=None; dt=DT/subdivisions
 for cycle in range(1,end+1):
  if cycle in commands: row=commands[cycle]
  if cycle==100001:
   currents[0]=0.; currents=project(currents,True)
  if cycle==150001:currents=project(currents,False)
  opened=100000<cycle<=150000
  disabled=200000<cycle<=250000 or cycle<85
  phase=(cycle-85)%4000
  terminals=[]
  for k in range(3):
   enable=(row['ccer']>>(4*k))&15
   width=row['ccr'+str(k+1)]*4
   if legacy:
    # Previous motor evaluator: complement regardless of the partner enable;
    # DTG=48 was spread symmetrically in timer-tick-normalized space.
    main=96<=phase<max(0,width-96)
    comp=min(4000,width+96)<=phase<3904
    on=main if enable==1 else comp if enable==4 else False
   else:
    # RM0351: for main-only or N-only, OCREF is routed without complement
    # or dead time. The fixture uses no channel with BOTH enables set.
    on=phase<width and enable!=0
   terminals.append(None if disabled or not on else V if enable==1 else 0.)
  for _ in range(subdivisions):
   angle=(theta*POLES)%TAU
   shapes=[shape(angle),shape(angle-2*math.pi/3),shape(angle+2*math.pi/3)]
   emf=[KE*omega*s for s in shapes]
   connected=[k for k in range(3) if terminals[k] is not None and not(opened and k==0)]
   neutral=sum(terminals[k]-R*currents[k]-emf[k] for k in connected)/len(connected) if len(connected)>=2 else 0.
   decay=math.exp(-dt*R/L)
   next_current=[0. if opened and k==0 else currents[k]+((terminals[k]-neutral-R*currents[k]-emf[k])/L)*dt if k in connected and len(connected)>=2 else currents[k]*decay for k in range(3)]
   currents=project(next_current,opened)
   if abs(omega)<=1e-9:torque=KT*sum(shapes[k]*currents[k] for k in range(3))
   else:torque=sum(emf[k]*currents[k] for k in range(3))/omega
   omega+=((torque-B*omega)/J)*dt
   theta+=omega*dt
   bus=sum(currents[k] for k in connected if terminals[k]==V)
  if first is None and any(currents):first={'cycle':cycle,'phase_cycle':phase,'terminals_v':terminals,'phase_currents_a':currents[:],'position_rad':theta,'speed_rpm':omega*60/TAU,'torque_nm':torque}
  if cycle in OBS:
   snapshots.append({'cycle':cycle,'position_rad':theta,'speed_rpm':omega*(60/TAU),'torque_nm':torque,'current_a':bus,'phase_currents_a':currents[:],'commutation_sector':min(5,math.floor(((theta*POLES)%TAU)/(TAU/6)))})
 return {'subdivisions':subdivisions,'end_cycle':end,'legacy_gates':legacy,'first_excitation':first,'samples':snapshots}

t0=time.monotonic()
coarse=run()
comparisons=[]
for expect,got in zip(trace['samples'],coarse['samples']):
 for field in ['position_rad','speed_rpm','torque_nm','current_a']:
  a=expect['motor'][field];b=got[field];tol=1e-12*max(1,abs(a))
  comparisons.append({'cycle':got['cycle'],'field':field,'difference':b-a,'tolerance':tol,'pass':abs(b-a)<=tol})
 for k,(a,b) in enumerate(zip(expect['motor']['phase_currents_a'],got['phase_currents_a'])):
  tol=1e-12*max(1,abs(a));comparisons.append({'cycle':got['cycle'],'field':'phase_currents_a['+str(k)+']','difference':b-a,'tolerance':tol,'pass':abs(b-a)<=tol})
fine=[run(n,100000) for n in [2,4,8]]
legacy=run(1,100000,True)
result={'contracts':['RM0351 Rev10 table199','crates/core/src/physics/motor/bldc.rs step() documentation','crates/core/src/physics/motor/shaft.rs step() documentation'],'capture_sha256':hashlib.sha256((ROOT/'current-wasm-trace.json').read_bytes()).hexdigest(),'reference_script_sha256':hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),'base':coarse,'comparisons':comparisons,'all_native_tolerance_pass':all(x['pass'] for x in comparisons),'fine':fine,'legacy_counterfactual_same_mmio':legacy,'duration_s':time.monotonic()-t0}
p=ROOT/'independent-replay-results.json';p.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'file':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'all_native_tolerance_pass':result['all_native_tolerance_pass'],'failures':[x for x in comparisons if not x['pass']],'base_first':coarse['first_excitation'],'sample0':coarse['samples'][0],'finer_sample0':[x['samples'][0] for x in fine],'legacy_sample0':legacy['samples'][0],'duration_s':result['duration_s']},indent=2))
