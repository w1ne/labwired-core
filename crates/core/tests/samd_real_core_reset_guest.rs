// SPDX-License-Identifier: MIT
//! Actual CODAL target_reset, invalid key, startup reentry and bounded batching.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cycle_clock::event_scheduler_enabled, system::cortex_m::configure_cortex_m,
    AdvanceRequest, Bus, Cpu, Machine, SimulationObserver,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    num::NonZeroU32,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const SOURCE: &str = "12d4759af0bf2b29726cfd79c487474a282cc5b0";
const CORE: &str = "2ed19d64ec3359143409a4b0a9758b4aeef3ee7d";
const BUDGET: u64 = 200_000;
const POST: u64 = 64;

#[derive(Debug, Default)]
struct State {
    step: u64,
    ended: u64,
    pc: u32,
    opcode: u32,
    regs: Vec<u32>,
    rows: Vec<Value>,
    returns: Vec<(u32, &'static str)>,
}
#[derive(Debug)]
struct Trace {
    state: Mutex<State>,
    symbols: BTreeMap<&'static str, u32>,
    ram: Vec<(u64, u64)>,
}
impl SimulationObserver for Trace {
    fn on_step_start(&self, pc: u32, opcode: u32) {
        let mut s = self.state.lock().unwrap();
        s.step += 1;
        s.pc = pc;
        s.opcode = opcode;
        let step = s.step;
        let regs = s.regs.clone();
        let returned: Vec<_> = s
            .returns
            .iter()
            .copied()
            .filter(|(ret, _)| *ret == pc)
            .collect();
        for (_, kind) in returned {
            s.rows
                .push(json!({"kind":kind,"step":step,"pc":pc,"opcode":opcode}));
        }
        s.returns.retain(|(ret, _)| *ret != pc);
        for (name, kind, ret) in [
            ("reset_entry", "entry", None),
            ("runtime_probe", "probe", None),
            ("target_reset", "target-call", Some("target-return")),
            (
                "runtime_reset_invalid_key",
                "invalid-call",
                Some("invalid-return"),
            ),
            (
                "runtime_core_reset_aircr_write",
                "writer-call",
                Some("writer-return"),
            ),
            ("runtime_core_reset_aircr_store", "request", None),
            ("target_panic", "panic", None),
        ] {
            if pc == self.symbols[name] {
                // At reset entry the previous-retirement registers are OLD
                // context. Do not publish them as a reset-entry snapshot.
                let mut row = json!({"kind":kind,"step":step,"pc":pc,"opcode":opcode});
                if kind != "entry" {
                    row["previous_retirement"] = json!({"r0":regs[0],"r1":regs[1],"sp":regs[13],"lr":regs[14],"xpsr":regs[16]});
                }
                s.rows.push(row);
                if let Some(ret) = ret {
                    s.returns.push((regs[14] & !1, ret));
                }
            }
        }
    }
    fn on_step_end(&self, _cycles: u32, registers: &[u32]) {
        let mut s = self.state.lock().unwrap();
        s.ended += 1;
        assert_eq!(registers.len(), 19);
        s.regs = registers.to_vec();
        if s.pc == self.symbols["reset_entry"] {
            let step = s.step;
            let pc = s.pc;
            // First startup instruction is LDR: it cannot modify SP/LR/PSR.
            s.rows.push(json!({"kind":"entry-registers","step":step,"pc":pc,
                "post_first_instruction":{"sp":registers[13],"lr":registers[14],"xpsr":registers[16],"pc":registers[15]}}));
        }
    }
    fn on_memory_write(&self, address: u64, _old: u8, value: u8) {
        if self.ram.iter().any(|&(a, b)| a <= address && address < b) {
            let mut s = self.state.lock().unwrap();
            let step = s.step;
            let pc = s.pc;
            s.rows.push(
                json!({"kind":"ram-byte","step":step,"pc":pc,"address":address,"value":value}),
            );
        }
    }
}

fn indices<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> {
    rows.iter().filter(|r| r["kind"] == kind).collect()
}
fn ordering(rows: &[Value]) {
    for (kind, count) in [
        ("entry", 2),
        ("entry-registers", 2),
        ("probe", 2),
        ("target-call", 1),
        ("invalid-call", 1),
        ("invalid-return", 1),
        ("writer-call", 2),
        ("writer-return", 1),
        ("request", 2),
        ("panic", 0),
    ] {
        assert_eq!(indices(rows, kind).len(), count, "closed {kind} census");
    }
    assert!(indices(rows, "target-return").is_empty());
    assert!(rows.iter().all(|r| [
        "entry",
        "entry-registers",
        "probe",
        "target-call",
        "invalid-call",
        "invalid-return",
        "writer-call",
        "writer-return",
        "request",
        "ram-byte"
    ]
    .contains(&r["kind"].as_str().unwrap())));
    let entries = indices(rows, "entry");
    let probes = indices(rows, "probe");
    let requests = indices(rows, "request");
    let at = |row: &Value| row["step"].as_u64().unwrap();
    for (row, value) in requests.iter().zip([4, 0x05fa0004u32]) {
        assert_eq!(row["opcode"], 0x6008); // STR r0,[r1] from the bound ELF.
        assert_eq!(row["previous_retirement"]["r0"], value);
        assert_eq!(row["previous_retirement"]["r1"], 0xe000ed0cu32);
    }
    assert!(at(entries[0]) < at(probes[0]));
    assert!(at(probes[0]) < at(indices(rows, "invalid-call")[0]));
    assert!(at(indices(rows, "invalid-call")[0]) < at(requests[0]));
    assert!(at(requests[0]) < at(indices(rows, "writer-return")[0]));
    assert!(at(indices(rows, "writer-return")[0]) < at(indices(rows, "invalid-return")[0]));
    assert!(at(indices(rows, "invalid-return")[0]) < at(indices(rows, "target-call")[0]));
    assert!(at(indices(rows, "target-call")[0]) < at(requests[1]));
    assert_eq!(
        at(entries[1]),
        at(requests[1]) + 1,
        "no old-context instruction after reset store"
    );
    assert!(at(entries[1]) < at(probes[1]));
    for row in indices(rows, "entry-registers") {
        assert_eq!(row["post_first_instruction"]["sp"], 0x20030000u32);
        assert_eq!(row["post_first_instruction"]["lr"], u32::MAX);
        assert_eq!(row["post_first_instruction"]["xpsr"], 0x01000000u32);
    }
}

#[test]
fn manufactured_reset_order_rejects_missing_returns_keys_and_old_context() {
    let mut rows = Vec::new();
    for (step, kind) in [
        (1, "entry"),
        (1, "entry-registers"),
        (2, "probe"),
        (3, "invalid-call"),
        (4, "writer-call"),
        (5, "request"),
        (6, "writer-return"),
        (7, "invalid-return"),
        (8, "target-call"),
        (9, "writer-call"),
        (10, "request"),
        (11, "entry"),
        (11, "entry-registers"),
        (12, "probe"),
    ] {
        let mut row = json!({"step":step,"kind":kind,"pc":1024,"opcode":0x6008});
        if kind == "request" {
            row["previous_retirement"] =
                json!({"r0":if step==5 {4}else{0x05fa0004u32},"r1":0xe000ed0cu32});
        }
        if kind == "entry-registers" {
            row["post_first_instruction"] =
                json!({"sp":0x20030000u32,"lr":u32::MAX,"xpsr":0x01000000u32});
        }
        rows.push(row);
    }
    ordering(&rows);
    for i in 0..rows.len() {
        let mut bad = rows.clone();
        bad.remove(i);
        assert!(std::panic::catch_unwind(|| ordering(&bad)).is_err());
    }
    for (index, pointer, value) in [
        (5, "/previous_retirement/r0", json!(0x05fa0004u32)),
        (10, "/previous_retirement/r1", json!(0xe000ed08u32)),
        (12, "/post_first_instruction/lr", json!(0)),
        (12, "/post_first_instruction/xpsr", json!(0xf1000000u32)),
    ] {
        let mut bad = rows.clone();
        *bad[index].pointer_mut(pointer).unwrap() = value;
        assert!(std::panic::catch_unwind(|| ordering(&bad)).is_err());
    }
    for kind in ["target-return", "writer-return", "unknown", "panic"] {
        let mut bad = rows.clone();
        bad.push(json!({"kind":kind,"step":13,"pc":1024}));
        assert!(std::panic::catch_unwind(|| ordering(&bad)).is_err());
    }
}

#[test]
#[ignore = "actual source-bound CODAL reset needs dedicated clean hosted build and both scheduler/batch profiles"]
fn actual_codal_core_reset_and_invalid_key() {
    let folder = PathBuf::from(std::env::var("LABWIRED_RESET_FIXTURE_DIR").unwrap());
    assert_eq!(std::env::var("LABWIRED_RESET_SOURCE").unwrap(), SOURCE);
    let provenance: Value =
        serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["source"], SOURCE);
    assert_eq!(provenance["core_source"], CORE);
    assert_eq!(provenance["caller_mode"], 19);
    assert_eq!(provenance["cpu_hz"], 120000000);
    assert_eq!(provenance["reset_scope"], "simulation-core-only");
    assert_eq!(provenance["reset_selector"], "BW_SIMULATION_CORE_RESET=1");
    assert_eq!(provenance["runtime_admission"], false);
    let path = folder.join("control.elf");
    let bytes = std::fs::read(&path).unwrap();
    let image = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(provenance["elf_sha256"], image);
    assert_eq!(
        provenance["map_sha256"],
        format!(
            "{:x}",
            Sha256::digest(std::fs::read(folder.join("control.map")).unwrap())
        )
    );
    let elf = goblin::elf::Elf::parse(&bytes).unwrap();
    let symbol = |name: &str| {
        let matches: Vec<_> = elf
            .syms
            .iter()
            .filter(|s| s.st_shndx != 0 && elf.strtab.get_at(s.st_name) == Some(name))
            .collect();
        assert_eq!(matches.len(), 1, "unique {name}");
        matches[0].st_value as u32
    };
    let symbols: BTreeMap<_, _> = [
        "reset_entry",
        "runtime_probe",
        "target_reset",
        "runtime_reset_invalid_key",
        "runtime_core_reset_aircr_write",
        "runtime_core_reset_aircr_store",
        "target_panic",
    ]
    .into_iter()
    .map(|name| (name, symbol(name) & !1))
    .collect();
    let data = (
        u64::from(symbol("_data_start")),
        u64::from(symbol("_data_end")),
    );
    let bss = (
        u64::from(symbol("_bss_start")),
        u64::from(symbol("_bss_end")),
    );
    let noinit = (
        u64::from(symbol("_noinit_start")),
        u64::from(symbol("_noinit_end")),
    );
    assert_eq!(noinit.1 - noinit.0, 48);
    assert_eq!(symbol("runtime_reset_persistent") as u64, noinit.0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut failed = 0;
    for quantum in [1u32, 64] {
        let chip =
            ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
        let mut manifest =
            SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
        // Core-local reset, not external-device reset. The full board's
        // NeoPixel resident requires per-cycle service and prevents the real
        // multi-instruction CPU-window adversary. Earlier fixtures stay wired.
        assert_eq!(manifest.external_devices.len(), 2);
        manifest.external_devices.clear();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut m = Machine::new(cpu, bus);
        m.load_firmware(&labwired_loader::load_elf(&path).unwrap())
            .unwrap();
        assert_eq!(symbol("runtime_vectors"), 0);
        assert_eq!(symbol("runtime_vectors_end"), 1024);
        assert_eq!(m.bus.read_u32(0).unwrap(), 0x20030000);
        for vector in 1..256 {
            let target = if vector == 1 {
                "reset_entry"
            } else if vector == 15 {
                "runtime_systick"
            } else {
                "runtime_fault"
            };
            assert_eq!(m.bus.read_u32(vector * 4).unwrap(), symbol(target) | 1);
        }
        let data_initial: Vec<_> = (0..data.1 - data.0)
            .map(|i| m.bus.read_u8(symbol("_data_load") as u64 + i).unwrap())
            .collect();
        for a in data.0..data.1 {
            m.bus.write_u8(a, 0x5a).unwrap();
        }
        for a in bss.0..bss.1 {
            m.bus.write_u8(a, 0xa5).unwrap();
        }
        assert!((noinit.0..noinit.1).all(|a| m.bus.read_u8(a).unwrap() == 0));
        assert_eq!(
            m.bus
                .read_u16(symbols["runtime_core_reset_aircr_store"] as u64)
                .unwrap(),
            0x6008
        );
        let mut regs = (0u8..16).map(|r| m.cpu.get_register(r)).collect::<Vec<_>>();
        regs.extend([m.cpu.xpsr, m.cpu.get_register(13), m.cpu.pc]);
        let trace = Arc::new(Trace {
            state: Mutex::new(State {
                regs,
                ..Default::default()
            }),
            symbols: symbols.clone(),
            ram: vec![data, bss, noinit],
        });
        m.add_observer(trace.clone());
        m.config.peripheral_tick_interval = quantum;
        m.config.idle_fast_forward_enabled = false;
        m.bus.config.observe_wide_ram_stores = true;
        let mut terminal = None;
        let mut total_batches = 0u64;
        let mut total_steps = 0u64;
        loop {
            let used = trace.state.lock().unwrap().step;
            let remaining = terminal.map(|t| t + POST - used).unwrap_or(BUDGET - used);
            if remaining == 0 {
                break;
            }
            let fuel = remaining.min(quantum.into());
            let request = if quantum == 1 {
                AdvanceRequest::single()
            } else {
                AdvanceRequest::run(Some(fuel)).with_batch_cap(NonZeroU32::new(quantum).unwrap())
            };
            let report = m.advance(request).unwrap();
            assert!(report.primary_steps > 0 && report.idle_cycles == 0);
            total_batches += report.cpu_batches;
            total_steps += report.primary_steps;
            let state = trace.state.lock().unwrap();
            if terminal.is_none() {
                // The terminal is the actual final status-byte store, not a
                // later host poll or a manufactured completion callback.
                terminal = state
                    .rows
                    .iter()
                    .find(|r| {
                        r["kind"] == "ram-byte"
                            && r["address"] == symbol("runtime_status") as u64 + 3
                            && r["value"] == 0
                            && m.bus.read_u32(symbol("runtime_status") as u64).unwrap() == 0x600d
                    })
                    .map(|r| r["step"].as_u64().unwrap());
                // On success find the LAST status word-store byte. BSS
                // clearance may also write a zero byte to this address.
                if terminal.is_some() {
                    terminal = state
                        .rows
                        .iter()
                        .rev()
                        .find(|r| {
                            r["kind"] == "ram-byte"
                                && r["address"] == symbol("runtime_status") as u64 + 3
                        })
                        .map(|r| r["step"].as_u64().unwrap());
                }
                if m.bus.read_u32(symbol("runtime_status") as u64).unwrap() & 0xf0000000
                    == 0xe0000000
                {
                    terminal = Some(state.step);
                }
            }
            if state.step >= BUDGET {
                break;
            }
        }
        let state = trace.state.lock().unwrap();
        assert_eq!(state.step, state.ended);
        assert_eq!(total_steps, state.step);
        let read = |name: &str| m.bus.read_u32(symbol(name) as u64).unwrap();
        let persistent: Vec<_> = (0..12)
            .map(|i| m.bus.read_u32(noinit.0 + i * 4).unwrap())
            .collect();
        let result = json!({"schema":1,"source":SOURCE,"core":CORE,"image":image,"provenance":provenance,
            "fixture":"pybadge-core-reset-without-external-residents","external_devices":manifest.external_devices.len(),
            "profile":if event_scheduler_enabled(){"event-scheduler"}else{"default"},"quantum":quantum,"budget":BUDGET,"post":POST,
            "steps":state.step,"terminal":terminal,"cpu_batches":total_batches,"status":read("runtime_status"),"stage":read("runtime_reset_stage"),
            "data":read("runtime_reset_data"),"bss":read("runtime_reset_bss"),"persistent":persistent,
            "final_core":{"primask":m.cpu.primask,"basepri":m.cpu.basepri,"faultmask":m.cpu.faultmask,"control":m.cpu.control,"exception":m.cpu.active_exception},
            "faults":[m.bus.read_u32(0xe000ed28).unwrap(),m.bus.read_u32(0xe000ed2c).unwrap()],
            "symbols":symbols,"data_range":[data.0,data.1],"bss_range":[bss.0,bss.1],"noinit_range":[noinit.0,noinit.1],"data_initial":data_initial,
            "addresses":{"before":symbol("runtime_reset_before"),"status":symbol("runtime_status"),"stage":symbol("runtime_reset_stage"),"data":symbol("runtime_reset_data"),"bss":symbol("runtime_reset_bss")},"rows":state.rows});
        for row in &state.rows {
            println!("SAMD_REAL_CORE_RESET_ROW {row}");
        }
        println!("SAMD_REAL_CORE_RESET_RESULT {result}");
        let valid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ordering(&state.rows);
            assert_eq!(read("runtime_status"), 0x600d);
            assert_eq!(read("runtime_reset_stage"), 4);
            assert_eq!(
                persistent,
                [
                    0x42575253,
                    2,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0x20030000,
                    0, // MRS xPSR masks EPSR; debugger snapshots above retain T.
                    u32::MAX,
                    1,
                    0
                ]
            );
            assert_eq!(read("runtime_reset_data"), 0x11223344);
            assert_eq!(read("runtime_reset_bss"), 0);
            assert_eq!(state.step, terminal.unwrap() + POST);
            assert!(state.step < BUDGET);
            assert!(m.cpu.primask);
            assert_eq!(m.cpu.basepri, 0);
            assert!(!m.cpu.faultmask);
            assert_eq!(m.cpu.control, 0);
            assert_eq!(m.cpu.active_exception, 0);
            assert_eq!(result["faults"], json!([0, 0]));
            if quantum == 64 {
                assert!(total_batches < total_steps / 2, "must actually batch");
            }
        }))
        .is_ok();
        if !valid {
            failed += 1;
        }
    }
    assert_eq!(failed, 0, "all reset/batch cases must pass");
}
