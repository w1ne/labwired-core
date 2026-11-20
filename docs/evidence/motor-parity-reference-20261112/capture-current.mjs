import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {initSync,WasmSimulator} from '/data/worktrees/labwired-analog-scope-clean-core-594a9931/packages/wasm-runtime/src/labwired_wasm.js';
const root='/data/worktrees/labwired-analog-scope-clean-core-594a9931';
const out='/data/work-evidence/motor-parity-review-594a9931-20261112';
const wasm=readFileSync(root+'/packages/playground/public/wasm/labwired_wasm_bg.wasm');
initSync({module:wasm});
const system=readFileSync(root+'/core/examples/nucleo-l476rg-bldc/system.yaml');
const chip=readFileSync(root+'/core/configs/chips/stm32l476.yaml');
const firmware=readFileSync(root+'/core/crates/wasm/tests/fixtures/firmware-l476-bldc-six-step.elf');
const sim=WasmSimulator.new_from_config(system.toString(),chip.toString(),firmware,undefined);
const norm=v=>v instanceof Map?Object.fromEntries([...v].map(([k,v])=>[k,norm(v)])):Array.isArray(v)?v.map(norm):v;
const refs=[['gpioa',8],['gpiob',13],['gpioa',9],['gpiob',14],['gpioa',10],['gpiob',15]].map(([peripheral,pin])=>({kind:'gpio',peripheral,pin}));
sim.watch_logic_signals(refs);
const samples=[], register_changes=[], startup=[];
const important=['cr1','cr2','psc','arr','ccer','ccmr1','ccmr2','ccr1','ccr2','ccr3','bdtr','phase_revision'];
let previous='', firstCurrent=null;
for(let cycle=1;cycle<=300000;cycle++) {
  const advanced=sim.step_batch(1); if(advanced!==1) throw new Error('unexpected cycle stride '+advanced);
  const timer=norm(sim.get_peripheral_snapshot('tim1'));
  const command=Object.fromEntries(important.map(k=>[k,timer[k]]));
  const sig=JSON.stringify(command);
  if(sig!==previous) {register_changes.push({cycle,pc:sim.get_pc(),...command,cnt:timer.cnt,psc_cnt:timer.psc_cnt});previous=sig;}
  if(cycle>=40160&&cycle<=40180) startup.push({cycle,timer:{cnt:timer.cnt,psc_cnt:timer.psc_cnt,ccer:timer.ccer},motor:sim.get_actuator_states()[0]});
  if(cycle===100000||cycle===150000||cycle===200000||cycle===250000||cycle===300000) {
    samples.push({cycle,motor:sim.get_actuator_states()[0]});
    if(cycle===100000) sim.set_motor_fault('drive_motor','open-phase-a',true);
    if(cycle===150000) {sim.set_motor_fault('drive_motor','open-phase-a',false);sim.set_motor_fault('drive_motor','hall-b-low',true);}
    if(cycle===200000) {sim.set_motor_fault('drive_motor','hall-b-low',false);sim.set_motor_fault('drive_motor','inverter',true);}
    if(cycle===250000) sim.set_motor_fault('drive_motor','inverter',false);
  }
}
const gates=norm(sim.read_logic_states(0));
sim.free();
const hash=b=>createHash('sha256').update(b).digest('hex');
const result={core:'a0b0a91c1868fe78e4af45e817e566807517ef1e',wasm_sha256:hash(wasm),system_sha256:hash(system),chip_sha256:hash(chip),firmware_sha256:hash(firmware),samples,register_changes,startup,gates};
const file=out+'/current-wasm-trace.json';writeFileSync(file,JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({file,sha256:hash(readFileSync(file)),register_changes:register_changes.length,gate_edges:gates.edges.length,dropped:gates.dropped,samples,first_nonzero:startup.find(v=>v.motor.phase_currents_a.some(x=>x!==0))},null,2));
