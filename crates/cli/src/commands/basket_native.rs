//! One persistent plant, two independently loaded Linux fixture epochs.
//! Only Machine/CosimSession advance time; no guest RAM/PC synchronization.
use super::Rv1103BasketArgs;
use crate::commands::basket_evidence::{
    validate_phase, Marker, MarkerTracker, PhaseProof, PHASE_IDS,
};
use labwired_core::{
    bus::SystemBus,
    cosim::{CosimAdvanceError, CosimSession, CosimSignalValue, CosimSignals},
    cpu::CortexA7System,
    memory::LinearMemory,
    system::{rv1103_basket::*, rv1103_linux::*},
    AdvanceRequest, AdvanceStop, Cpu, Machine,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

type Guest = Machine<CortexA7System>;
struct PhaseStart {
    index: usize,
    action_cycle: u64,
    from_seq: u64,
    console_start: usize,
    evicted_at_action: u64,
    peer_before: Vec<String>,
    inspect_before: Value,
}
struct Fixture {
    machine: Guest,
    sink: Arc<Mutex<Vec<u8>>>,
    console: Vec<u8>,
    line_start: usize,
    tracker: MarkerTracker,
    active: Option<PhaseStart>,
    phases: Vec<Value>,
    startup: Value,
    acceptance: Option<Value>,
}
impl Fixture {
    fn load(path: &Path, receipt: &mut Value) -> Result<Self, String> {
        let artifacts = BasketArtifacts::read(path).map_err(|e| e.to_string())?;
        receipt["preparation_provenance"] = artifacts.manifest.clone();
        receipt["preparation_provenance_classification"]="bounded caller-reported tool/path/log observations; runtime identities independently matched reviewed trusted profile".into();
        receipt["actual_input_sha256"] = json!(artifacts
            .artifacts
            .iter()
            .map(|(k, v)| (k.clone(), sha256(v)))
            .collect::<BTreeMap<_, _>>());
        let modules= ["i2c-dev.ko","i2c-rk3x.ko"].into_iter()
            .map(|name| {
                let bytes=&artifacts.artifacts[name];
                let prefix=b"vermagic=";
                let start=bytes.windows(prefix.len()).position(|w|w==prefix)
                    .ok_or_else(|| format!("missing actual {name} vermagic"))?+prefix.len();
                let end=bytes[start..].iter().position(|b|*b==0).ok_or("unterminated vermagic")?+start;
                let vermagic=std::str::from_utf8(&bytes[start..end]).map_err(|e|e.to_string())?;
                if !vermagic.starts_with("6.18.56-0-lts ") {return Err("module vermagic kernel mismatch".into());}
                Ok((name.to_owned(),json!({"sha256":sha256(bytes),"vermagic":vermagic,"load":"ordinary insmod, no force flags in pinned init"})))
            }).collect::<Result<BTreeMap<_,_>,String>>()?;
        receipt["modules"] = json!(modules);
        let mut bus = SystemBus::empty();
        if !bus.cycle_clock.scheduler_enabled() {
            return Err("basket requires event-scheduler".into());
        }
        bus.cpu_hz = FUNCTIONAL_CPU_HZ;
        bus.ram = LinearMemory::new(RAM_BYTES as usize, 0);
        let loaded = load_basket(&mut bus, artifacts).map_err(|e| e.to_string())?;
        let sink = Arc::new(Mutex::new(Vec::new()));
        if !bus.attach_uart_tx_sink_named("uart2", sink.clone(), false) {
            return Err("UART2 sink attachment failed".into());
        }
        let mut machine = Machine::new(loaded.cpu, bus);
        machine.config.idle_fast_forward_enabled = true;
        if machine.cpu.get_pc() != 0
            || !machine.supply_status().held_in_reset
            || !machine.supply_status().routed
            || machine.supply_status().release_volts != Some(3.0)
            || machine.supply_status().reset_volts != Some(2.7)
        {
            return Err(
                "strict fixture must start held with monitor and unstarted native PC0".into(),
            );
        }
        Ok(Self {
            machine,
            sink,
            console: Vec::new(),
            line_start: 0,
            tracker: MarkerTracker::default(),
            active: None,
            phases: Vec::new(),
            startup: Value::Null,
            acceptance: None,
        })
    }
    fn drain(&mut self, limit: usize) -> Result<(), String> {
        let mut bytes = self.sink.lock().map_err(|e| e.to_string())?;
        let remaining = limit.saturating_sub(self.console.len());
        let overflow = bytes.len() > remaining;
        self.console.extend(bytes.drain(..).take(remaining));
        if overflow {
            bytes.clear();
            return Err("full bounded console reached byte limit; excess output refused".into());
        }
        Ok(())
    }
    fn inspection(&self) -> Value {
        serde_json::to_value(self.machine.inspect(None, &Default::default()))
            .expect("inspect serialization")
    }
    fn meta(&self, id: &str) -> Result<Value, String> {
        let inspected = self.machine.inspect(None, &Default::default());
        let device = inspected
            .devices
            .iter()
            .find(|d| d.id == id && d.declared)
            .ok_or("canonical declared device missing")?;
        device
            .artifacts
            .first()
            .map(|a| a.meta.clone())
            .ok_or_else(|| format!("{id} lacks actual artifact"))
    }
    fn peer(&self) -> Result<Vec<String>, String> {
        self.machine
            .bus
            .device_logs("wifi")
            .and_then(|logs| {
                logs.into_iter()
                    .find(|l| l.name == "esp_at")
                    .map(|l| l.lines())
            })
            .ok_or_else(|| "canonical peer log missing".into())
    }
    fn snapshot(&self) -> Value {
        json!({"console":String::from_utf8_lossy(&self.console),"startup":self.startup,
            "phases":self.phases,"acceptance":self.acceptance,
            "local_cycles":self.machine.total_cycles,"last_pc":format!("{:#010x}",self.machine.cpu.get_pc()),
            "idle_cycles":self.machine.idle_fast_forward_cycles_skipped,"supply":self.machine.supply_status(),
            "inspection":self.inspection(),"latest_seq":self.machine.bus.bus_trace.latest_seq(),
            "evicted":self.machine.bus.bus_trace.evicted(),"last_trace":self.machine.bus.bus_trace.snapshot(),
            "active_phase":self.active.as_ref().map(|p|json!({"index":p.index,"action_cycle":p.action_cycle,
                "from_seq":p.from_seq,"peer_before":p.peer_before,"inspect_before":p.inspect_before}))})
    }
}

struct Driver<'a> {
    args: &'a Rv1103BasketArgs,
    started: Instant,
    progress_at: Instant,
    retired: u64,
    fuel: u64,
    global: u64,
    external_off: bool,
    raw5: f64,
    raw3: f64,
    process: Option<Value>,
    plant_steps: Vec<Value>,
    actions: Vec<Value>,
}
fn signals_json(s: &CosimSignals) -> Value {
    json!(s
        .iter()
        .map(|(k, v)| {
            let v = match v {
                CosimSignalValue::Bool(v) => json!({"kind":"Bool","value":v}),
                CosimSignalValue::F64(v) => json!({"kind":"F64","value":v}),
                CosimSignalValue::I64(v) => json!({"kind":"I64","value":v}),
                CosimSignalValue::Text(v) => json!({"kind":"Text","value":v}),
            };
            (k.clone(), v)
        })
        .collect::<BTreeMap<_, _>>())
}
fn owned_plant_process() -> Result<Value, String> {
    let owner = std::process::id();
    let mut children = std::collections::BTreeSet::new();
    for task in std::fs::read_dir(format!("/proc/{owner}/task")).map_err(|e| e.to_string())? {
        let path = task.map_err(|e| e.to_string())?.path().join("children");
        // A test worker may own the spawn. Union only this process's tasks.
        if let Ok(text) = std::fs::read_to_string(path) {
            for pid in text.split_whitespace() {
                children.insert(pid.parse::<u32>().map_err(|e| e.to_string())?);
            }
        }
    }
    let pids: Vec<_> = children.into_iter().collect();
    if pids.len() != 1 {
        return Err(format!(
            "strict one-plant process expected one owned child, got {pids:?}"
        ));
    }
    let pid = pids[0];
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| e.to_string())?;
    let (_, fields) = stat.rsplit_once(") ").ok_or("invalid actual child stat")?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let start_ticks = fields
        .get(19)
        .ok_or("child starttime missing")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let parent = fields
        .get(1)
        .ok_or("child parent missing")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).map_err(|e| e.to_string())?;
    let args: Vec<_> = cmd
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();
    if parent != owner
        || args.len() != 2
        || !args[1].ends_with("/smart-basket-power/models/power.py")
    {
        return Err("owned child is not the canonical persistent Python plant".into());
    }
    Ok(json!({"pid":pid,"owner":owner,"start_ticks":start_ticks,"argv":args}))
}
fn numeric(session: &CosimSession, path: &str) -> Result<f64, String> {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(v)) if v.is_finite() => Ok(*v),
        Some(CosimSignalValue::I64(v)) => Ok(*v as f64),
        _ => Err(format!("actual plant missing numeric {path}")),
    }
}
fn boolean(session: &CosimSession, path: &str, expected: bool) -> Result<(), String> {
    if session.signals().get(path) == Some(&CosimSignalValue::Bool(expected)) {
        Ok(())
    } else {
        Err(format!("actual typed plant {path} != {expected}"))
    }
}
impl Driver<'_> {
    fn apply_rails(&mut self, fixture: &mut Fixture) -> Result<(), String> {
        if ![0., 5.].contains(&self.raw5) || ![0., 3.3].contains(&self.raw3) {
            return Err("closed synthetic proxy rejects unexpected actual raw rails".into());
        }
        fixture
            .machine
            .set_supply_volts(if self.raw5 == 5. { 3.3 } else { 0. });
        fixture
            .machine
            .bus
            .update_supply_domain(
                "external3v3",
                if self.external_off { 0. } else { self.raw3 },
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn window(
        &mut self,
        fixture: &mut Fixture,
        session: &mut CosimSession,
    ) -> Result<bool, String> {
        if self.started.elapsed().as_secs() >= self.args.wall_seconds {
            return Err("bounded wall timeout".into());
        }
        if self.global >= self.args.max_cycles
            || self.retired >= self.args.max_retirements
            || self.fuel >= self.args.max_fuel
        {
            return Err("bounded aggregate cycle/retirement/fuel limit".into());
        }
        let fuel = 4096
            .min(self.args.max_retirements - self.retired)
            .min(self.args.max_fuel - self.fuel);
        let request = AdvanceRequest::run(Some(fuel))
            .with_cycle_limit(4096.min(self.args.max_cycles - self.global))
            .with_batch_cap(NonZeroU32::new(64).unwrap());
        let result = session.advance(&mut fixture.machine, request);
        // Preserve all bounded console even if native execution/model failed.
        let drain = fixture.drain(self.args.console_bytes);
        self.global = session
            .global_cycles(fixture.machine.total_cycles)
            .map_err(|e| e.to_string())?;
        let advance=match result {
            Ok(advance)=>advance,
            Err(CosimAdvanceError::Model{report,error})=>{
                self.retired+=report.primary_steps; self.fuel+=report.fuel_consumed;
                return Err(format!("actual model boundary failed after committed machine report: {error}; console drain: {drain:?}"));
            },
            Err(error)=>return Err(format!("{error}; failed native window accounting may partially progress; console drain: {drain:?}"))
        };
        self.retired += advance.report.primary_steps;
        self.fuel += advance.report.fuel_consumed;
        drain?;
        if advance.report.stop == AdvanceStop::NoProgress {
            return Err("Machine NoProgress".into());
        }
        if !advance.new_routing_errors.is_empty() {
            return Err(format!(
                "actual route errors: {:?}",
                advance.new_routing_errors
            ));
        }
        if self.progress_at.elapsed().as_secs() >= 30 {
            eprintln!("basket progress: wall={:.1}s global_cycles={} retirements={} fuel={} pc={:#010x} console_bytes={} last_line={}",
                self.started.elapsed().as_secs_f64(),self.global,self.retired,self.fuel,fixture.machine.cpu.get_pc(),
                fixture.console.len(),String::from_utf8_lossy(&fixture.console).lines().last().unwrap_or(""));
            self.progress_at = Instant::now();
        }
        let stepped = !advance.routed.is_empty();
        if stepped {
            let process = owned_plant_process()?;
            if let Some(first) = &self.process {
                if first != &process {
                    return Err(
                        "persistent actual plant child identity changed across boundaries/fixtures"
                            .into(),
                    );
                }
            } else {
                self.process = Some(process);
            }

            if advance.routed.len() != 1 || advance.routed[0].model_id != "basket_power" {
                return Err("unexpected canonical plant step count/identity".into());
            }
            self.raw5 = numeric(session, "basket.power.rail_5v")?;
            self.raw3 = numeric(session, "basket.power.rail_3v3")?;
            self.apply_rails(fixture)?;
            self.plant_steps.push(json!({"global_cycles":self.global,"local_cycles":fixture.machine.total_cycles,
                "time_ns":labwired_core::cosim::routing::cycles_to_ns(self.global,FUNCTIONAL_CPU_HZ),
                "signals":signals_json(session.signals()),"raw5":self.raw5,"raw3":self.raw3,
                "effective_external3v3":if self.external_off {0.} else {self.raw3},
                "external_override":self.external_off,"supply":fixture.machine.supply_status(),
                "applied":"after actual plant boundary, before next guest window"}));
        }
        Ok(stepped)
    }
    fn next_boundary(
        &mut self,
        fixture: &mut Fixture,
        session: &mut CosimSession,
    ) -> Result<(), String> {
        loop {
            if self.window(fixture, session)? {
                return Ok(());
            }
        }
    }
    fn startup(&mut self, f: &mut Fixture, s: &mut CosimSession) -> Result<(), String> {
        let held_start = self.global;
        let local_start = f.machine.total_cycles;
        let retired_start = self.retired;
        let fuel_start = self.fuel;
        let ram = sha256(&f.machine.bus.ram.data);
        let before = f.machine.supply_status();
        self.next_boundary(f, s)?;
        if self.retired != retired_start
            || f.machine.cpu.get_pc() != 0
            || self.raw5 != 5.
            || self.raw3 != 3.3
        {
            return Err("actual first healthy plant step must follow held startup with zero CPU retirements".into());
        }
        let local = f.machine.total_cycles;
        let release = f
            .machine
            .advance(AdvanceRequest::run(Some(0)))
            .map_err(|e| e.to_string())?;
        if release.fuel_consumed != 0
            || release.primary_steps != 0
            || release.elapsed_cycles != 0
            || f.machine.total_cycles != local
            || f.machine.cpu.get_pc() != 0
            || sha256(&f.machine.bus.ram.data) != ram
            || f.machine.supply_status().power_on_resets != 1
        {
            return Err("legitimate zero-fuel supply release mutated accounting/RAM or failed unstarted PC0".into());
        }
        f.machine
            .cpu
            .initialize_linux(KERNEL_ADDRESS as u32, DTB_ADDRESS as u32)
            .map_err(|e| e.to_string())?;
        if f.machine.cpu.get_pc() != KERNEL_ADDRESS as u32 {
            return Err(
                "validated final zImage entry not initialized before first instruction".into(),
            );
        }
        f.startup = json!({"global_start":held_start,"global_end":self.global,"local_start":local_start,
            "local_end":local,"held_cycles":self.global-held_start,"held_nanoseconds":labwired_core::cosim::routing::cycles_to_ns(self.global-held_start,FUNCTIONAL_CPU_HZ),
            "held_retirements":self.retired-retired_start,"held_fuel":self.fuel-fuel_start,
            "before":before,"after":f.machine.supply_status(),"zero_fuel_release":format!("{release:?}"),
            "ram_sha256_before_and_after_release":ram,"entry_pc":f.machine.cpu.get_pc(),
            "r0":f.machine.cpu.get_register(0),"r1":f.machine.cpu.get_register(1),"r2":f.machine.cpu.get_register(2),
            "sequence":"held native PC0 -> actual plant boundary -> functional supply release via zero-fuel Machine.advance -> validated Linux boot regs before first instruction"});
        if held_start == 0 && self.global - held_start != 2_400_000 {
            return Err("first startup must be actual 100ms/2.4M cycles".into());
        }
        Ok(())
    }
    fn phase_action(&mut self, f: &mut Fixture, index: usize) -> Result<(), String> {
        if index != 2 {
            let inputs = if index == 0 {
                [1., -0.5, 0.25]
            } else {
                [-0.25, 0.5, -1.]
            };
            for (channel, value) in ["x", "y", "z"].into_iter().zip(inputs) {
                f.machine
                    .set_input_on("accelerometer", channel, value)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.external_off = index == 2;
        self.apply_rails(f)?;
        let before = f.inspection();
        let action = PhaseStart {
            index,
            action_cycle: f.machine.total_cycles,
            from_seq: f.machine.bus.bus_trace.latest_seq(),
            console_start: f.console.len(),
            evicted_at_action: f.machine.bus.bus_trace.evicted(),
            peer_before: f.peer()?,
            inspect_before: before.clone(),
        };
        self.actions.push(json!({"phase":PHASE_IDS[index],"local_cycle":action.action_cycle,
            "global_cycle":self.global,"from_seq":action.from_seq,"evicted_at_action":action.evicted_at_action,"raw5":self.raw5,"raw3":self.raw3,
            "effective_external3v3":if self.external_off {0.} else {self.raw3},
            "provenance":if self.external_off {"explicit carrier external-rail override"} else {"actual persistent plant rails"},
            "inspection":before,"synchronization":"host observes exact READY after <=4096-cycle advance; actual prepared helper waits 100ms"}));
        eprintln!(
            "basket host action: phase={} global_cycles={}",
            PHASE_IDS[index], self.global
        );
        f.active = Some(action);
        Ok(())
    }
    fn phase_done(&mut self, f: &mut Fixture, index: usize) -> Result<(), String> {
        let start = f.active.take().ok_or("DONE lacks actual host actuation")?;
        if start.index != index {
            return Err("host phase mismatch".into());
        }
        let ring = f.machine.bus.bus_trace.snapshot();
        let through = f.machine.bus.bus_trace.latest_seq();
        let proof = PhaseProof {
            action_cycle: start.action_cycle,
            from_seq: start.from_seq,
            through_seq: through,
            first_retained_seq: ring.first().map(|e| e.seq).unwrap_or(through + 1),
            evicted: f.machine.bus.bus_trace.evicted(),
            events: ring
                .into_iter()
                .filter(|e| e.seq > start.from_seq)
                .collect(),
            peer_before: start.peer_before,
            peer_after: f.peer()?,
            sensor: f.meta("accelerometer")?,
            radio: f.meta("wifi")?,
            raw5: self.raw5,
            raw3: self.raw3,
            effective3: if self.external_off { 0. } else { self.raw3 },
            host_held: f.machine.supply_status().held_in_reset,
        };
        let text = String::from_utf8_lossy(&f.console[start.console_start..]);
        let result = validate_phase(index, &text, &proof);
        f.phases.push(
            json!({"phase":PHASE_IDS[index],"status":if result.is_ok() {"PASS"} else {"FAIL"},
            "stop":result.as_ref().err(),"proof":proof,"evicted_at_action":start.evicted_at_action,"inspection_before":start.inspect_before,
            "inspection_after":f.inspection(),"console":text}),
        );
        // Restore must reach a fresh power generation, preserving input B.
        if index == 3 {
            let off = f.phases.get(2).ok_or("missing external off proof")?;
            if f.phases[3]["proof"]["sensor"]["supply_generation"].as_u64()
                <= off["proof"]["sensor"]["supply_generation"].as_u64()
                || f.phases[3]["proof"]["radio"]["supply_generation"].as_u64()
                    <= off["proof"]["radio"]["supply_generation"].as_u64()
            {
                return Err("restoration lacks actual fresh device supply generation".into());
            }
        }
        result
    }
    fn boot(&mut self, f: &mut Fixture, s: &mut CosimSession) -> Result<(), String> {
        loop {
            self.window(f, s)?;
            let text = String::from_utf8_lossy(&f.console);
            if text.contains("Kernel panic")
                || text.contains("LW_BASKET_FAILURE")
                || text.contains("LW_LINUX_BOOT_FAILURE")
            {
                return Err("actual Linux failure or kernel panic".into());
            }
            while let Some(relative) = f.console[f.line_start..].iter().position(|b| *b == b'\n') {
                let end = f.line_start + relative;
                let line = std::str::from_utf8(&f.console[f.line_start..end])
                    .map_err(|e| e.to_string())?
                    .trim_end_matches('\r')
                    .to_owned();
                f.line_start = end + 1;
                match f.tracker.consume(&line)? {
                    Some(Marker::Ready(index)) => self.phase_action(f, index)?,
                    Some(Marker::Done(index)) => self.phase_done(f, index)?,
                    Some(Marker::Accepted) => {
                        if f.phases.len() != 4 {
                            return Err(
                                "guest success lacks four independent actual phase proofs".into()
                            );
                        }
                        f.acceptance = Some(guest_identity(&String::from_utf8_lossy(&f.console))?);
                        return Ok(());
                    }
                    None => {}
                }
            }
        }
    }
    fn fault_recovery(
        &mut self,
        f: &mut Fixture,
        s: &mut CosimSession,
        receipt: &mut Value,
    ) -> Result<(), String> {
        let soc_before = numeric(s, "basket.power.soc")?;
        let time_before = self.global;
        s.set_signal("basket.fault_short", CosimSignalValue::Bool(true))
            .map_err(|e| e.to_string())?;
        self.next_boundary(f, s)?;
        boolean(s, "basket.power.cutoff_latched", true)?;
        if self.raw5 != 0. || self.raw3 != 0. || !f.machine.supply_status().held_in_reset {
            return Err("actual short fault failed to hold host and shut rails".into());
        }
        let cutoff = json!({"global_cycle":self.global,"signals":signals_json(s.signals()),"supply":f.machine.supply_status(),
            "inspection":f.inspection()});
        let from = f.machine.bus.bus_trace.latest_seq();
        let retired = self.retired;
        let fuel = self.fuel;
        let off_start = self.global;
        let peer = f.peer()?;
        self.next_boundary(f, s)?;
        let ring = f.machine.bus.bus_trace.snapshot();
        let through = f.machine.bus.bus_trace.latest_seq();
        let events: Vec<_> = ring.iter().filter(|e| e.seq > from).cloned().collect();
        let complete = through >= from
            && ring.first().map(|e| e.seq).unwrap_or(through + 1) <= from + 1
            && events.len() as u64 == through - from
            && events
                .iter()
                .enumerate()
                .all(|(i, e)| e.seq == from + 1 + i as u64);
        let off = json!({"global_start":off_start,"global_end":self.global,"retirements":self.retired-retired,
            "fuel":self.fuel-fuel,"from_seq":from,"through_seq":through,"first_retained_seq":ring.first().map(|e|e.seq),
            "evicted":f.machine.bus.bus_trace.evicted(),"events":events,"complete_coverage":complete,"peer_before":peer,"peer_after":f.peer()?,
            "supply":f.machine.supply_status(),"inspection":f.inspection()});
        receipt["fault"] = json!({"soc_before":soc_before,"global_before":time_before,"cutoff":cutoff,"held_advance":off});
        if self.retired != retired
            || self.global <= off_start
            || !complete
            || f.peer()? != peer
            || events.iter().any(|e| e.bus == "i2c3" || e.bus == "uart4")
        {
            return Err(
                "actual fault held advance lacks zero retirement/full no-inner-traffic proof"
                    .into(),
            );
        }
        s.set_signal("basket.fault_short", CosimSignalValue::Bool(false))
            .map_err(|e| e.to_string())?;
        self.next_boundary(f, s)?;
        receipt["fault"]["clear_fault_alone"] = json!({"global_cycle":self.global,"signals":signals_json(s.signals()),"supply":f.machine.supply_status()});
        boolean(s, "basket.power.cutoff_latched", true)?;
        if self.raw5 != 0. || self.raw3 != 0. || !f.machine.supply_status().held_in_reset {
            return Err("clearing short alone improperly cleared persistent latch".into());
        }
        s.set_signal("basket.reset", CosimSignalValue::Bool(true))
            .map_err(|e| e.to_string())?;
        self.next_boundary(f, s)?;
        receipt["fault"]["explicit_healthy_reset"] = json!({"global_cycle":self.global,"signals":signals_json(s.signals()),
            "supply":f.machine.supply_status(),"policy":"clear latch only, discard old held fixture without advancing pending release into native PC0"});
        boolean(s, "basket.power.cutoff_latched", false)?;
        if self.raw5 != 5. || self.raw3 != 3.3 || numeric(s, "basket.power.soc")? > soc_before {
            return Err("explicit reset failed healthy rails or rewound battery state".into());
        }
        Ok(())
    }
}

fn guest_identity(console: &str) -> Result<Value, String> {
    let base = crate::commands::rv1103_linux::acceptance(console)
        .ok_or("base actual PID1/mount/kernel/uptime gates failed")?;
    let exact = |required: &str| {
        console
            .lines()
            .filter(|l| l.trim_end_matches('\r') == required)
            .count()
            == 1
    };
    if console
        .lines()
        .filter(|l| l.trim_end_matches('\r').starts_with("LW_BASKET_INSMOD"))
        .count()
        != 2
    {
        return Err(
            "ordinary actual insmod record set is not exactly the two pinned modules".into(),
        );
    }
    for required in [
        "LW_BASKET_INSMOD module=i2c-rk3x exit=0",
        "LW_BASKET_INSMOD module=i2c-dev exit=0",
    ] {
        if !exact(required) {
            return Err("ordinary actual insmod success identity/count missing".into());
        }
    }
    let lines: Vec<_> = console.lines().map(|l| l.trim_end_matches('\r')).collect();
    let taints: Vec<_> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("LW_BASKET_TAINT="))
        .collect();
    if taints.len() != 1 {
        return Err("actual kernel taint evidence missing/duplicate".into());
    }
    if taints[0].is_empty() || !taints[0].bytes().all(|b| b.is_ascii_digit()) {
        return Err("actual kernel taint evidence is not ASCII decimal".into());
    }
    let taint: u64 = taints[0].parse().map_err(|_| "invalid taint")?;
    // External modules and unsigned modules may taint. Force-load/removal is
    // refused, along with unexpected taints; never claim this kernel untainted.
    if taint & !(4096 | 8192) != 0 {
        return Err(format!(
            "unexpected/forced kernel module taint bits {taint}"
        ));
    }
    let sysfs: Vec<_> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("LW_BASKET_SYSFS i2c="))
        .collect();
    if sysfs.len() != 1 {
        return Err("actual sysfs evidence missing/duplicate".into());
    }
    let (i2c, rest) = sysfs[0]
        .split_once(" uart=")
        .ok_or("invalid sysfs fields")?;
    let (uart, dt) = rest
        .split_once(" uart_dt=")
        .ok_or("invalid sysfs DT field")?;
    if i2c != "/sys/devices/platform/ff460000.i2c/i2c-3"
        || dt != "/sys/firmware/devicetree/base/serial@ff4e0000"
        || uart.len() > 512
    {
        return Err("actual physical I2C3/UART4 sysfs identity mismatch".into());
    }
    let segments: Vec<_> = uart.split('/').collect();
    let canonical_id = |s: &str| {
        !s.is_empty()
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s == "0" || !s.starts_with('0'))
            && s.len() <= 10
            && s.parse::<u32>().is_ok_and(|n| n <= i32::MAX as u32)
    };
    if segments.len() != 7 || segments[..5] != ["", "sys", "devices", "platform", "ff4e0000.serial"]
    {
        return Err("actual Linux serial parent ancestry mismatch".into());
    }
    let c = segments[5]
        .strip_prefix("ff4e0000.serial:")
        .ok_or("serial controller name")?;
    let port = segments[6]
        .strip_prefix("ff4e0000.serial:")
        .ok_or("serial port name")?;
    let (pc, p) = port.split_once('.').ok_or("serial port IDs")?;
    if !canonical_id(c) || !canonical_id(p) || pc != c {
        return Err("actual serial controller/port canonical IDs mismatch".into());
    }
    Ok(json!({"pid":1,"kernel":"6.18.56-0-lts","machine":"armv7l",
        "uptime_before":base.uptime_before,"uptime_after":base.uptime_after,
        "taint":taint,"allowed_taint_bits":{"external_module":4096,"unsigned_module":8192},
        "force_load_or_remove":false,"i2c_sysfs":i2c,"uart_sysfs":uart,"uart_of_node":dt,
        "modules":"ordinary insmod exit0, exact pinned module hashes/vermagic; no force invocation"}))
}

fn record_sources(receipt: &mut Value) -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let compiled_sources = [
        (
            "crates/cli/src/commands/rv1103_basket.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/cli/src/commands/rv1103_basket.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/cli/src/commands/basket_native.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/cli/src/commands/basket_native.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/cli/src/commands/basket_evidence.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/cli/src/commands/basket_evidence.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/cli/src/commands/rv1103_linux.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/cli/src/commands/rv1103_linux.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/cli/src/dispatch.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/cli/src/dispatch.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/cli/src/commands/mod.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/cli/src/commands/mod.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/system/rv1103_basket.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/system/rv1103_basket.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/system/rv1103_linux.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/system/rv1103_linux.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/cosim/routing.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/cosim/routing.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/cosim/external_process.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/cosim/external_process.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/cosim/registry.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/cosim/registry.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/lib.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/lib.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/peripherals/components/declarative_i2c.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/peripherals/components/declarative_i2c.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/peripherals/components/esp32c3_esp_at.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/peripherals/components/esp32c3_esp_at.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/peripherals/components/supply_domain.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/peripherals/components/supply_domain.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/peripherals/armv7/rk3x_i2c.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/peripherals/armv7/rk3x_i2c.rs"
            ))
            .as_slice(),
        ),
        (
            "crates/core/src/peripherals/armv7/dw_uart.rs",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/core/src/peripherals/armv7/dw_uart.rs"
            ))
            .as_slice(),
        ),
        (
            "examples/luckfox-pico-mini-a-linux/basket/trusted-prepared.json",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/luckfox-pico-mini-a-linux/basket/trusted-prepared.json"
            ))
            .as_slice(),
        ),
        (
            "examples/luckfox-pico-mini-a-linux/basket/trusted-newc.json",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/luckfox-pico-mini-a-linux/basket/trusted-newc.json"
            ))
            .as_slice(),
        ),
        (
            "examples/luckfox-pico-mini-a-linux/basket/phases.json",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/luckfox-pico-mini-a-linux/basket/phases.json"
            ))
            .as_slice(),
        ),
        (
            "examples/luckfox-pico-mini-a-linux/basket/init",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/luckfox-pico-mini-a-linux/basket/init"
            ))
            .as_slice(),
        ),
        (
            "examples/luckfox-pico-mini-a-linux/basket/helper.c",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/luckfox-pico-mini-a-linux/basket/helper.c"
            ))
            .as_slice(),
        ),
        (
            "examples/smart-basket-power/models/power.py",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/smart-basket-power/models/power.py"
            ))
            .as_slice(),
        ),
        (
            "tools/cosim/smart_basket_power.py",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tools/cosim/smart_basket_power.py"
            ))
            .as_slice(),
        ),
    ];
    let mut sources = BTreeMap::new();
    for (path, compiled) in compiled_sources {
        let bytes =
            std::fs::read(root.join(path)).map_err(|e| format!("actual source {path}: {e}"))?;
        let hash = sha256(compiled);
        sources.insert(path.to_owned(), hash.clone());
        receipt["compiled_source_sha256"] = json!(sources);
        if sha256(&bytes) != hash {
            return Err(format!("source changed after compiled candidate: {path}"));
        }
    }
    let head = std::process::Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| e.to_string())?;
    let head = std::str::from_utf8(&head.stdout)
        .map_err(|e| e.to_string())?
        .trim();
    if head.len() != 40 || !head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("actual LabWired source HEAD unavailable".into());
    }
    receipt["labwired_source_head"] = head.into();
    receipt["compiled_critical_source_set_sha256"] =
        sha256(&serde_json::to_vec(&sources).expect("source map")).into();
    let export: Value = serde_json::from_str(CANONICAL_EXPORT).map_err(|e| e.to_string())?;
    receipt["export_provenance"] = export.clone();
    let app_root = root
        .join("examples/luckfox-pico-mini-a-linux/basket-native-runtime")
        .join(
            export["canonical_root"]
                .as_str()
                .ok_or("export canonical root missing")?,
        );
    let mut core_authority = BTreeMap::new();
    for entry in export["core_sources"]
        .as_array()
        .ok_or("export core sources missing")?
    {
        let path = entry["path"].as_str().ok_or("export core path missing")?;
        let bytes = std::fs::read(root.join(path)).map_err(|e| e.to_string())?;
        let actual = sha256(&bytes);
        if entry["sha256"].as_str() != Some(&actual) {
            return Err(format!(
                "actual core authority differs from reviewed export: {path}"
            ));
        }
        core_authority.insert(path.to_owned(), actual);
    }
    receipt["actual_core_authority_sha256"] = json!(core_authority);
    let mut app_sources = BTreeMap::new();
    for entry in export["sources"]
        .as_array()
        .ok_or("export sources missing")?
    {
        let path = entry["path"].as_str().ok_or("export source path missing")?;
        let bytes = std::fs::read(app_root.join(path))
            .map_err(|e| format!("actual app source {path}: {e}"))?;
        let actual = sha256(&bytes);
        if entry["sha256"].as_str() != Some(&actual) {
            return Err(format!(
                "actual canonical app source differs from reviewed export: {path}"
            ));
        }
        app_sources.insert(path.to_owned(), actual);
    }
    receipt["actual_source_sha256"] = json!(sources);
    receipt["actual_app_source_sha256"] = json!(app_sources);
    receipt["source_identity_note"]="runtime source hashes equal compiler-captured source bytes; frozen candidate receipt independently binds CLI/backend/build command/source tree".into();
    Ok(root)
}

pub(super) fn execute(args: &Rv1103BasketArgs, receipt: &mut Value) -> Result<(), String> {
    let mut driver = Driver {
        args,
        started: Instant::now(),
        progress_at: Instant::now(),
        retired: 0,
        fuel: 0,
        global: 0,
        external_off: false,
        raw5: 0.,
        raw3: 0.,
        process: None,
        plant_steps: Vec::new(),
        actions: Vec::new(),
    };
    let mut live: Option<Fixture> = None;
    let mut session: Option<CosimSession> = None;
    let mut fixtures = Vec::new();
    let result = (|| {
        if !cfg!(feature = "event-scheduler") {
            return Err("RV1103 basket requires event-scheduler native feature".into());
        }
        let root = record_sources(receipt)?;
        live = Some(Fixture::load(&args.artifacts, receipt)?);
        let manifest: labwired_config::SystemManifest =
            serde_json::from_str(CANONICAL_MANIFEST).map_err(|e| e.to_string())?;
        let base = root.join("examples/luckfox-pico-mini-a-linux/basket-native-runtime");
        session = CosimSession::new(
            &manifest.cosim_models,
            &base,
            &live.as_ref().unwrap().machine.bus,
        )
        .map_err(|e| e.to_string())?;
        let s = session
            .as_mut()
            .ok_or("canonical persistent CosimSession missing")?;
        if s.model_count() != 1
            || s.uses_fallback_clock()
            || s.cpu_hz() != FUNCTIONAL_CPU_HZ
            || s.step_ns() != 100_000_000
            || !s.binding_errors().is_empty()
        {
            return Err(
                "canonical persistent plant/explicit clock/binding contract mismatch".into(),
            );
        }
        let exported: Value = serde_json::from_str(CANONICAL_EXPORT).map_err(|e| e.to_string())?;
        for (path, typed) in exported["initial_signals"]
            .as_object()
            .ok_or("initial typed signals missing")?
        {
            let value = match typed["kind"].as_str() {
                Some("Bool") => {
                    CosimSignalValue::Bool(typed["value"].as_bool().ok_or("Bool value missing")?)
                }
                Some("F64") => {
                    CosimSignalValue::F64(typed["value"].as_f64().ok_or("F64 value missing")?)
                }
                _ => return Err("canonical signal has invalid scalar kind".into()),
            };
            s.set_signal(path, value).map_err(|e| e.to_string())?;
        }
        receipt["initial_signals"] = signals_json(s.signals());
        let f = live.as_mut().unwrap();
        driver.startup(f, s)?;
        driver.boot(f, s)?;
        fixtures.push(f.snapshot());
        driver.fault_recovery(f, s, receipt)?;
        let epoch = driver.global;
        let continuity = signals_json(s.signals());
        // A new bus/native CPU is a fixture epoch, not a ROM reboot of old RAM.
        live = None;
        live = Some(Fixture::load(&args.artifacts, receipt)?);
        s.rebind_fixture_epoch(&live.as_ref().unwrap().machine.bus, epoch)
            .map_err(|e| e.to_string())?;
        s.set_signal("basket.reset", CosimSignalValue::Bool(false))
            .map_err(|e| e.to_string())?;
        receipt["recovery_epoch"] = json!({"epoch_cycles":epoch,"signals_before_rebind":continuity,
            "signals_after_rebind_and_reset_false":signals_json(s.signals()),"models":s.model_count(),
            "contract":"same CosimSession/runner/child/battery; fresh strict bus/native CPU; exact monotonic global offset and fresh router bindings"});
        let f = live.as_mut().unwrap();
        driver.startup(f, s)?;
        driver.boot(f, s)?;
        fixtures.push(f.snapshot());
        Ok(())
    })();
    if let Some(f) = live.as_mut() {
        let _ = f.drain(args.console_bytes);
        receipt["console"] = String::from_utf8_lossy(&f.console).to_string().into();
        receipt["current_fixture"] = f.snapshot();
    }
    receipt["fixtures"] = json!(fixtures);
    receipt["plant_steps"] = json!(driver.plant_steps);
    receipt["host_actions"] = json!(driver.actions);
    receipt["accounting"] = json!({"global_cycles":driver.global,"retirements_completed_windows":driver.retired,
        "fuel_completed_windows":driver.fuel,"wall_elapsed_seconds":driver.started.elapsed().as_secs_f64(),
        "partial_error_policy":"completed windows only; a failing native/model boundary may partially progress"});
    if let Some(s) = &session {
        receipt["final_signals"] = signals_json(s.signals());
    }
    receipt["persistent_process"] = json!(driver.process);
    drop(session.take());
    if let Some(process) = &driver.process {
        let pid = process["pid"].as_u64().expect("observed PID");
        let reaped = !Path::new(&format!("/proc/{pid}")).exists();
        receipt["owned_process_reaped"] = reaped.into();
        if !reaped {
            return Err(
                "owned persistent plant process was not reaped after terminal cleanup".into(),
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    fn identity_parser_control(taint: &str) -> String {
        format!(
            "Linux version 6.18.56-0-lts (parser control)\n\
             Run /init as init process\n\
             LW_LINUX_MOUNTS proc=ok sysfs=ok devtmpfs=ok\n\
             LW_LINUX_PID1 pid=1 kernel=6.18.56-0-lts machine=armv7l\n\
             processor : 0\nCPU architecture: 7\nMemTotal: 44400 kB\n\
             LW_LINUX_UPTIME_BEFORE=12.59\nLW_LINUX_UPTIME_AFTER=13.63\n\
             LW_LINUX_BOOT_ACCEPTED\n\
             LW_BASKET_INSMOD module=i2c-rk3x exit=0\n\
             LW_BASKET_INSMOD module=i2c-dev exit=0\n\
             LW_BASKET_TAINT={taint}\n\
             LW_BASKET_SYSFS i2c=/sys/devices/platform/ff460000.i2c/i2c-3 uart=/sys/devices/platform/ff4e0000.serial/ff4e0000.serial:0/ff4e0000.serial:0.0 uart_dt=/sys/firmware/devicetree/base/serial@ff4e0000\n"
        )
    }
    #[test]
    fn guest_identity_parser_accepts_only_declared_external_and_unsigned_taint_subsets() {
        for taint in ["0", "4096", "8192", "12288"] {
            let identity = guest_identity(&identity_parser_control(taint)).unwrap();
            assert_eq!(identity["taint"], taint.parse::<u64>().unwrap());
            assert_eq!(identity["force_load_or_remove"], false);
        }
    }
    #[test]
    fn guest_identity_parser_rejects_force_unexpected_malformed_and_duplicate_evidence() {
        // These are parser controls, not evidence of a running Linux guest.
        for taint in [
            "2",
            "8",
            "12290",
            "12296",
            "1",
            "16384",
            "",
            "-1",
            "+12288",
            "12x",
            " 12288",
            "18446744073709551616",
        ] {
            assert!(
                guest_identity(&identity_parser_control(taint)).is_err(),
                "taint={taint:?}"
            );
        }
        let accepted = identity_parser_control("12288");
        assert!(guest_identity(&(accepted + "LW_BASKET_TAINT=12288\n")).is_err());
    }
    #[test]
    fn guest_identity_parser_rejects_duplicate_conflicting_or_unreviewed_module_markers() {
        let accepted = identity_parser_control("12288");
        for duplicate in [
            "LW_BASKET_INSMOD module=i2c-rk3x exit=0\n",
            "LW_BASKET_INSMOD module=i2c-dev exit=0\n",
            "LW_BASKET_INSMOD module=i2c-rk3x exit=1\n",
            "LW_BASKET_INSMOD module=unreviewed exit=0\n",
        ] {
            assert!(
                guest_identity(&(accepted.clone() + duplicate)).is_err(),
                "extra marker={duplicate:?}"
            );
        }
    }
    #[test]
    #[ignore = "requires LABWIRED_RV1103_BASKET_ARTIFACTS reviewed prepared inputs"]
    fn actual_persistent_plant_startup_hold_release_and_fresh_epoch_keep_one_child_and_time() {
        let args = Rv1103BasketArgs::parse_from([
            "basket",
            "--artifacts",
            &std::env::var("LABWIRED_RV1103_BASKET_ARTIFACTS").unwrap(),
            "--receipt",
            "unused",
        ]);
        let mut receipt = json!({});
        record_sources(&mut receipt)
            .expect("actual compiler-captured/core/app/model source authority preflight");
        let mut f = Fixture::load(&args.artifacts, &mut receipt).unwrap();
        let configs: labwired_config::SystemManifest =
            serde_json::from_str(CANONICAL_MANIFEST).unwrap();
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/luckfox-pico-mini-a-linux/basket-native-runtime");
        let mut s = CosimSession::new(&configs.cosim_models, &base, &f.machine.bus)
            .unwrap()
            .unwrap();
        let export: Value = serde_json::from_str(CANONICAL_EXPORT).unwrap();
        for (path, typed) in export["initial_signals"].as_object().unwrap() {
            s.set_signal(
                path,
                match typed["kind"].as_str().unwrap() {
                    "Bool" => CosimSignalValue::Bool(typed["value"].as_bool().unwrap()),
                    "F64" => CosimSignalValue::F64(typed["value"].as_f64().unwrap()),
                    _ => panic!("unexpected canonical scalar"),
                },
            )
            .unwrap();
        }
        let mut d = Driver {
            args: &args,
            started: Instant::now(),
            progress_at: Instant::now(),
            retired: 0,
            fuel: 0,
            global: 0,
            external_off: false,
            raw5: 0.,
            raw3: 0.,
            process: None,
            plant_steps: Vec::new(),
            actions: Vec::new(),
        };
        d.startup(&mut f, &mut s).unwrap();
        assert_eq!(d.global, 2_400_000);
        assert_eq!(d.retired, 0);
        assert_eq!(f.machine.cpu.get_pc(), KERNEL_ADDRESS as u32);
        assert_eq!(f.startup["held_cycles"], 2_400_000);
        assert_eq!(f.startup["held_fuel"], 2_400_000);
        assert_eq!(f.startup["held_nanoseconds"], 100_000_000);
        assert_eq!(f.startup["r2"], DTB_ADDRESS as u32);
        let process = d.process.clone();
        let soc = numeric(&s, "basket.power.soc").unwrap();
        // Unit control: hold this unexecuted fixture explicitly, then exercise
        // actual short/latch/reset plant boundaries without any Linux claim.
        f.machine.set_supply_volts(0.);
        d.fault_recovery(&mut f, &mut s, &mut receipt).unwrap();
        assert_eq!(d.retired, 0);
        assert_eq!(receipt["fault"]["held_advance"]["retirements"], 0);
        let epoch = d.global;
        drop(f);
        let mut fresh = Fixture::load(&args.artifacts, &mut receipt).unwrap();
        s.rebind_fixture_epoch(&fresh.machine.bus, epoch).unwrap();
        s.set_signal("basket.reset", CosimSignalValue::Bool(false))
            .unwrap();
        d.startup(&mut fresh, &mut s).unwrap();
        assert_eq!(d.global, epoch + 2_400_000);
        assert_eq!(d.retired, 0);
        assert_eq!(d.process, process);
        assert!(numeric(&s, "basket.power.soc").unwrap() < soc);
        assert_eq!(fresh.machine.total_cycles, 2_400_000);
        assert_eq!(fresh.machine.cpu.get_pc(), KERNEL_ADDRESS as u32);
        assert_eq!(s.model_count(), 1);
        let pid = d.process.unwrap()["pid"].as_u64().unwrap();
        drop(s);
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }
}
