// SPDX-License-Identifier: MIT
//! Actual CODAL clock-gated timeout/cleanup; not complete TX/native Arcade admission.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, peripherals::spi::SpiDevice, system::cortex_m::configure_cortex_m, Bus,
    Machine, SimulationObserver,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct Row {
    step: usize,
    kind: &'static str,
    value: u32,
}
#[derive(Debug, Default)]
struct Trace {
    step: Mutex<usize>,
    rows: Mutex<Vec<Row>>,
    gate: u64,
    gate_mask: u8,
    timeout_active: Mutex<bool>,
    gate_seen: Mutex<bool>,
}
impl Trace {
    fn record(&self, kind: &'static str, value: u32) {
        self.rows.lock().unwrap().push(Row {
            step: *self.step.lock().unwrap(),
            kind,
            value,
        });
    }
}
impl SimulationObserver for Trace {
    fn on_memory_write(&self, address: u64, _old: u8, value: u8) {
        if *self.timeout_active.lock().unwrap() && address == self.gate {
            if value & self.gate_mask == 0 {
                *self.gate_seen.lock().unwrap() = true;
            }
            self.record(
                if value & self.gate_mask == 0 {
                    "clock-off"
                } else {
                    "clock-on"
                },
                u32::from(value),
            );
        }
        if *self.timeout_active.lock().unwrap()
            && *self.gate_seen.lock().unwrap()
            && address == 0x41008098
            && value & 128 != 0
        {
            self.record("gpio-release", 1);
        }
        if address == 0x43000028 {
            self.record("accept", u32::from(value));
        }
    }
}
struct Slave(Arc<Trace>);
impl SpiDevice for Slave {
    fn cs_pin(&self) -> &str {
        "PB7"
    }
    fn cs_select(&mut self) {
        self.0.record("select", 0);
    }
    fn cs_release(&mut self) {
        self.0.record("release", 0);
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.0.record("wire", u32::from(byte));
        0xff
    }
}
fn ordering(rows: &[Row]) {
    let indices = |kind| {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let select = indices("select");
    let release = indices("release");
    let accept = indices("accept");
    let wire = indices("wire");
    let dma = indices("dma");
    let txc = indices("txc");
    let callback = indices("callback");
    let returned = indices("callback-return");
    let resumed = indices("async-resume");
    let blocked = indices("blocking-return");
    assert_eq!(
        [
            select.len(),
            release.len(),
            accept.len(),
            wire.len(),
            dma.len(),
            txc.len(),
            callback.len(),
            returned.len(),
            resumed.len(),
            blocked.len()
        ],
        [2, 2, 8, 8, 2, 2, 1, 1, 1, 1]
    );
    assert_eq!(
        accept.iter().map(|i| rows[*i].value).collect::<Vec<_>>(),
        [0x12, 0x34, 0x56, 0x78].repeat(2)
    );
    assert_eq!(
        wire.iter().map(|i| rows[*i].value).collect::<Vec<_>>(),
        [0x12, 0x34, 0x56, 0x78].repeat(2)
    );
    for phase in 0..2 {
        assert!(select[phase] < accept[phase * 4] && accept[phase * 4 + 3] < dma[phase]);
        assert!(
            dma[phase] < wire[phase * 4 + 3]
                && wire[phase * 4 + 3] < txc[phase]
                && txc[phase] < release[phase]
        );
        for byte in phase * 4..phase * 4 + 4 {
            assert!(accept[byte] < wire[byte]);
        }
    }
    assert!(
        release[0] < callback[0]
            && callback[0] < returned[0]
            && returned[0] < resumed[0]
            && resumed[0] < select[1]
    );
    assert!(release[1] < blocked[0]);
    assert!(rows.first().unwrap().step > 0 && rows.windows(2).all(|r| r[0].step <= r[1].step));
}
#[test]
fn manufactured_missing_recovery_ordering_refuses() {
    let mut rows = Vec::new();
    for phase in 0..2 {
        rows.push(Row {
            step: rows.len() + 1,
            kind: "select",
            value: 0,
        });
        for (i, value) in [0x12, 0x34, 0x56, 0x78].into_iter().enumerate() {
            rows.push(Row {
                step: rows.len() + 1,
                kind: "accept",
                value,
            });
            if i == 3 {
                rows.push(Row {
                    step: rows.len() + 1,
                    kind: "dma",
                    value: 0,
                });
            }
            rows.push(Row {
                step: rows.len() + 1,
                kind: "wire",
                value,
            });
        }
        for kind in ["txc", "release"] {
            rows.push(Row {
                step: rows.len() + 1,
                kind,
                value: 0,
            });
        }
        for kind in if phase == 0 {
            vec!["callback", "callback-return", "async-resume"]
        } else {
            vec!["blocking-return"]
        } {
            rows.push(Row {
                step: rows.len() + 1,
                kind,
                value: 0,
            });
        }
    }
    ordering(&rows);
    for index in 0..rows.len() {
        let mut bad = rows.clone();
        bad.remove(index);
        assert!(std::panic::catch_unwind(|| ordering(&bad)).is_err());
    }
    let mut bad = rows.clone();
    let dma = bad.iter().position(|r| r.kind == "dma").unwrap();
    let final_wire = bad
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == "wire")
        .nth(3)
        .unwrap()
        .0;
    bad.swap(dma, final_wire);
    assert!(std::panic::catch_unwind(|| ordering(&bad)).is_err());
}

fn timeout_ordering(rows: &[Row], remaining: u16, gate_mask: u8) {
    let ix = |kind| {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let unique = |kind| {
        let found = ix(kind);
        assert_eq!(found.len(), 1, "unique {kind}");
        found[0]
    };
    let select = unique("select");
    let release = unique("release");
    let gpio = unique("gpio-release");
    assert_eq!(rows[gpio].value, 1);
    let worker = unique("gate-worker");
    let failure = unique("failure");
    let returned = unique("failure-return");
    let resume = unique("timeout-resume");
    let waited_begin = unique("wait-begin");
    let waited_end = unique("wait-end");
    let accept = ix("accept");
    let wire = ix("wire");
    let on = ix("clock-on");
    let off = ix("clock-off");
    assert!(!off.is_empty() && on.len() == off.len() + 1);
    assert_eq!(
        rows.len(),
        9 + accept.len() + wire.len() + on.len() + off.len()
    );
    assert!(remaining > 0 && remaining < 32);
    assert_eq!(accept.len(), 32 - usize::from(remaining));
    assert!(wire.len() <= accept.len());
    let payload = [0x91, 0x82, 0x73, 0x64]
        .into_iter()
        .chain(std::iter::repeat(0))
        .take(32)
        .collect::<Vec<_>>();
    for (n, i) in accept.iter().enumerate() {
        assert_eq!(rows[*i].value, payload[n]);
        assert!(select < *i && *i < off[0]);
    }
    for (n, i) in wire.iter().enumerate() {
        assert_eq!(rows[*i].value, payload[n]);
        assert!(accept[n] < *i && *i < off[0]);
    }
    for n in 0..off.len() {
        assert!(on[n] < off[n] && off[n] < on[n + 1]);
        assert_eq!(rows[off[n]].value & u32::from(gate_mask), 0);
    }
    for i in &on {
        assert_ne!(rows[*i].value & u32::from(gate_mask), 0);
    }
    assert!(on[0] < select && worker < off[0] && off[0] < gpio && gpio < failure);
    // GPIO released while gated. The modeled slave sees that release only
    // when a clocked tick resolves CS; preserve this distinct raw observation.
    let restored_for_release = *on.iter().find(|i| **i > gpio).unwrap();
    assert!(restored_for_release < release && release < returned);
    assert!(failure < *on.last().unwrap() && *on.last().unwrap() < returned);
    assert!(
        failure < returned
            && returned < resume
            && resume < waited_begin
            && waited_begin < waited_end
    );
    assert!(
        rows[waited_end]
            .value
            .wrapping_sub(rows[waited_begin].value)
            >= 1499 * 120
    );
    assert!(rows.windows(2).all(|r| r[0].step <= r[1].step));
}
#[test]
fn manufactured_timeout_ordering_refuses() {
    let kinds = [
        ("clock-on", 1),
        ("select", 0),
        ("accept", 0x91),
        ("accept", 0x82),
        ("gate-worker", 0x20002200),
        ("clock-off", 0),
        ("gpio-release", 1),
        ("failure", 0x20001000),
        ("clock-on", 1),
        ("release", 0),
        ("failure-return", 0x800),
        ("timeout-resume", 0x20002000),
        ("wait-begin", 120000),
        ("wait-end", 300000),
    ];
    let rows = kinds
        .into_iter()
        .enumerate()
        .map(|(i, (kind, value))| Row {
            step: i + 1,
            kind,
            value,
        })
        .collect::<Vec<_>>();
    timeout_ordering(&rows, 30, 1);
    for i in 0..rows.len() {
        let mut bad = rows.clone();
        bad.remove(i);
        assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    }
    let mut bad = rows.clone();
    bad[13].value = 120001;
    assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    let mut bad = rows.clone();
    bad[6].value = 0; // The actual GPIO value must identify release.
    assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    let mut bad = rows.clone();
    bad.swap(6, 7);
    assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    let mut bad = rows.clone();
    bad[2].value = 255;
    assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    assert!(std::panic::catch_unwind(|| timeout_ordering(&rows, 29, 1)).is_err());
    let mut bad = rows.clone();
    bad[5].value = 1;
    assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    let mut bad = rows.clone();
    bad.insert(
        6,
        Row {
            step: 6,
            kind: "wire",
            value: 0x91,
        },
    );
    assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    let mut gclk = rows.clone();
    for row in &mut gclk {
        if row.kind == "clock-on" {
            row.value = 64;
        }
    }
    timeout_ordering(&gclk, 30, 64);
    gclk[5].value = 64;
    assert!(std::panic::catch_unwind(|| timeout_ordering(&gclk, 30, 64)).is_err());
    // Declared exclusive cleanup temporarily restores the clock to retire SPI,
    // then retains the caller's old gate before delivering failure. Do not hide
    // either edge or the earlier clock-dependent slave CS notification.
    let repaired = [
        ("clock-on", 1),
        ("select", 0),
        ("accept", 0x91),
        ("accept", 0x82),
        ("gate-worker", 0x20002200),
        ("clock-off", 0),
        ("gpio-release", 1),
        ("clock-on", 1),
        ("release", 0),
        ("clock-off", 0),
        ("failure", 0x20001000),
        ("clock-on", 1),
        ("failure-return", 0x800),
        ("timeout-resume", 0x20002000),
        ("wait-begin", 120000),
        ("wait-end", 300000),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (kind, value))| Row {
        step: i + 1,
        kind,
        value,
    })
    .collect::<Vec<_>>();
    timeout_ordering(&repaired, 30, 1);
    for i in 0..repaired.len() {
        let mut bad = repaired.clone();
        bad.remove(i);
        assert!(std::panic::catch_unwind(|| timeout_ordering(&bad, 30, 1)).is_err());
    }
}
#[test]
#[ignore = "actual source-bound CODAL clock-gated timeout callers requires dedicated hosted build and both scheduler profiles"]
fn actual_codal_spi_clock_gated_timeout_and_recovery() {
    let folder = PathBuf::from(std::env::var("LABWIRED_SPI_FIXTURE_DIR").unwrap());
    let source = std::env::var("LABWIRED_SPI_SOURCE").unwrap();
    assert!(source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()));
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["source"], source);
    let mode: u32 = std::env::var("LABWIRED_SPI_MODE").unwrap().parse().unwrap();
    assert!([10, 11].contains(&mode));
    assert_eq!(provenance["caller_mode"], mode);
    assert_eq!(
        provenance["core_source"],
        "2ed19d64ec3359143409a4b0a9758b4aeef3ee7d"
    );
    assert_eq!(provenance["runtime_admission"], false);
    assert_eq!(provenance["cpu_hz"], 120000000);
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
    let symbol = |name| {
        let matching: Vec<_> = elf
            .syms
            .iter()
            .filter(|s| s.st_shndx != 0 && elf.strtab.get_at(s.st_name) == Some(name))
            .collect();
        assert_eq!(matching.len(), 1, "unique symbol {name}");
        matching[0].st_value
    };
    let names = [
        "runtime_status",
        "runtime_checks",
        "runtime_heap_reused",
        "runtime_switches",
        "runtime_restores",
        "runtime_clock_irqs",
        "runtime_spi_stage",
        "runtime_spi_callbacks",
        "runtime_spi_callback_ipsr",
        "runtime_spi_callback_cs",
        "runtime_spi_argument",
        "runtime_spi_reentry_result",
        "runtime_spi_main_identity",
        "runtime_spi_worker_identity",
        "runtime_spi_blocking_returned",
    ];
    let addresses = names.map(symbol);
    let timeout_names = [
        "runtime_spi_timeout_stage",
        "runtime_spi_failures",
        "runtime_spi_failure_result",
        "runtime_spi_failure_ipsr",
        "runtime_spi_failure_mask",
        "runtime_spi_failure_cs",
        "runtime_spi_failure_argument",
        "runtime_spi_failure_reentry",
        "runtime_spi_gate_worker",
        "runtime_spi_failure_fiber",
        "runtime_spi_gate_saved",
        "runtime_spi_gate_held",
        "runtime_spi_gate_restored",
        "runtime_spi_cleanup_spi",
        "runtime_spi_cleanup_inten",
        "runtime_spi_cleanup_channel",
        "runtime_spi_gate_remaining",
        "runtime_spi_timeout_remaining",
        "runtime_spi_gate_cycles",
        "runtime_spi_timeout_cycles",
        "runtime_spi_cleanup_cycles",
        "runtime_spi_gate_busy",
        "runtime_spi_after_wait",
        "runtime_spi_wait_begin_cycles",
        "runtime_spi_wait_end_cycles",
        "runtime_spi_wait_begin_us",
        "runtime_spi_wait_end_us",
    ];
    let timeout_addresses = timeout_names.map(symbol);
    let bss = symbol("_bss_start")..symbol("_bss_end");
    let data = symbol("_data_start")..symbol("_data_end");
    let mut distinct = [addresses.to_vec(), timeout_addresses.to_vec()].concat();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), names.len() + timeout_names.len());
    assert!(addresses
        .iter()
        .chain(&timeout_addresses)
        .all(|a| bss.contains(a) && a % 4 == 0));
    assert!(data.start >= 0x20000000 && data.end <= bss.start && bss.end < 0x2002e000);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    chip.peripherals.push(serde_yaml::from_str("id: tx-dma\ntype: sam_dmac_sercom4_tx_byte\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\nclock: { controller: mclk, reg: AHBMASK, bit: 9 }\n").unwrap());
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let trace = Arc::new(Trace {
        gate: if mode == 10 { 0x40000820 } else { 0x40001d08 },
        gate_mask: if mode == 10 { 1 } else { 64 },
        timeout_active: Mutex::new(true),
        ..Trace::default()
    });
    bus.attach_spi_device("sercom4", Box::new(Slave(trace.clone())))
        .unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.add_observer(trace.clone());
    machine
        .load_firmware(&labwired_loader::load_elf(&path).unwrap())
        .unwrap();
    assert_eq!(symbol("runtime_vectors"), 0);
    assert_eq!(symbol("runtime_vectors_end"), 1024);
    assert_eq!(machine.bus.read_u32(0).unwrap(), 0x20030000);
    for vector in 1..256 {
        let word = machine.bus.read_u32(vector * 4).unwrap();
        let target = match vector {
            1 => "reset_entry",
            15 => "runtime_systick",
            47 => "runtime_dma_irq",
            79 => "runtime_spi_irq",
            _ => "runtime_fault",
        };
        assert_eq!(word & 1, 1);
        assert_eq!(u64::from(word & !1), symbol(target) & !1);
    }
    let expected_data: Vec<_> = (0..data.end - data.start)
        .step_by(4)
        .map(|offset| machine.bus.read_u32(symbol("_data_load") + offset).unwrap())
        .collect();
    for a in bss.clone().step_by(4) {
        machine.bus.write_u32(a, 0xa5a5a5a5).unwrap();
    }
    for a in data.clone().step_by(4) {
        machine.bus.write_u32(a, 0x5a5a5a5a).unwrap();
    }

    let mut startup = false;
    let mut previous = 0;
    let mut entries = [0; 3];
    let mut returns = [0; 3];
    let mut swaps = 0;
    let mut restores = 0;
    let mut callback_return = None;
    let mut failure_return = None;
    let mut callback_seen = false;
    let mut callback_returned = false;
    let mut failure_seen = false;
    let mut failure_returned = false;
    let mut worker_seen = false;
    let mut timeout_resumed = false;
    let mut wait_seen = false;
    let mut wait_started = false;
    let mut wait_finished = false;
    let mut resumed = false;
    let mut blocked = false;
    let mut recovery_start = None;
    let mut terminal = None;
    let mut diagnostic_stage = 0;
    for step in 1..=4_000_000 {
        *trace.step.lock().unwrap() = step;
        let pc = machine.cpu.pc & !1;
        if startup && pc == symbol("runtime_spi_failed") as u32 & !1 {
            assert!(!failure_seen);
            failure_seen = true;
            assert_eq!(machine.cpu.active_exception, 15);
            assert!(!machine.cpu.primask);
            assert_eq!(machine.cpu.r1, 3);
            failure_return = Some(machine.cpu.lr & !1);
            trace.record("failure", machine.cpu.r0);
        }
        if startup && failure_seen && !failure_returned && Some(pc) == failure_return {
            failure_returned = true;
            assert_eq!(machine.cpu.active_exception, 15);
            assert!(!machine.cpu.primask);
            trace.record("failure-return", pc);
        }
        if startup && pc == symbol("runtime_spi_done") as u32 & !1 {
            assert!(!callback_seen);
            callback_seen = true;
            assert_eq!(machine.cpu.active_exception, 79);
            callback_return = Some(machine.cpu.lr & !1);
            trace.record("callback", machine.cpu.r0);
        }
        if startup && callback_seen && !callback_returned && Some(pc) == callback_return {
            callback_returned = true;
            assert_eq!(machine.cpu.active_exception, 79);
            trace.record("callback-return", pc);
        }
        if pc == symbol("swap_context") as u32 & !1 {
            swaps += 1;
        }
        if pc == symbol("restore_register_context") as u32 & !1 {
            restores += 1;
        }
        if let Err(error) = machine.step() {
            println!("SAMD_REAL_SPI_TIMEOUT_STEP_ERROR mode={mode} step={step} source={source} elf_sha256={image} pc={:#x} error={error:?}",machine.cpu.pc);
            for row in trace.rows.lock().unwrap().iter() {
                println!(
                    "SAMD_REAL_SPI_TIMEOUT_FAILURE_ROW mode={mode} step={} kind={} value={}",
                    row.step, row.kind, row.value
                );
            }
            panic!("actual timeout guest must not fault: {error:?}");
        }
        if !startup && machine.cpu.pc & !1 == symbol("runtime_probe") as u32 & !1 {
            for a in bss.clone().step_by(4) {
                assert_eq!(machine.bus.read_u32(a).unwrap(), 0);
            }
            for (a, expected) in data.clone().step_by(4).zip(&expected_data) {
                assert_eq!(machine.bus.read_u32(a).unwrap(), *expected);
            }
            assert!(machine.cpu.primask && machine.cpu.active_exception == 0);
            startup = true;
        }
        let exception = machine.cpu.active_exception;
        assert!([0, 15, 47, 79].contains(&exception));
        if exception != previous {
            if previous != 0 {
                returns[[15, 47, 79].iter().position(|x| *x == previous).unwrap()] += 1;
            }
            if exception != 0 {
                entries[[15, 47, 79].iter().position(|x| *x == exception).unwrap()] += 1;
                if exception == 47 {
                    let wb = machine.bus.read_u32(0x4100a038).unwrap();
                    assert!(bss.contains(&u64::from(wb)) && wb % 16 == 0);
                    assert_eq!(machine.bus.read_u16(u64::from(wb + 2)).unwrap(), 0);
                    assert_eq!(machine.bus.read_u8(0x4100a04e).unwrap() & 3, 2);
                    trace.record("dma", 0);
                } else if exception == 79 {
                    assert!(machine.bus.read_u8(0x43000018).unwrap() & 2 != 0);
                    trace.record("txc", 0);
                }
            }
        }
        previous = exception;
        if !startup {
            continue;
        }
        let v = addresses.map(|a| machine.bus.read_u32(a).unwrap());
        let f = timeout_addresses.map(|a| machine.bus.read_u32(a).unwrap());
        if f[8] != 0 && !worker_seen {
            worker_seen = true;
            trace.record("gate-worker", f[8]);
        }
        if f[0] == 4 && !timeout_resumed {
            timeout_resumed = true;
            trace.record("timeout-resume", v[12]);
        }
        if f[23] != 0 && !wait_started {
            wait_started = true;
            trace.record("wait-begin", f[23]);
        }
        if f[24] != 0 && !wait_finished {
            wait_finished = true;
            trace.record("wait-end", f[24]);
        }
        if f[22] == 1 && !wait_seen {
            wait_seen = true;
            recovery_start = Some(trace.rows.lock().unwrap().len());
            *trace.timeout_active.lock().unwrap() = false;
        }
        if f[0] != diagnostic_stage || step == 4_000_000 || ![0, 0x100, 0x600d].contains(&v[0]) {
            diagnostic_stage = f[0];
            println!("SAMD_REAL_SPI_TIMEOUT_DIAGNOSTIC mode={mode} step={step} source={source} elf_sha256={image} pc={:#x} mask={} exception={exception} witnesses={v:?} timeout={f:?} irq_entries={entries:?} irq_returns={returns:?} swaps={swaps} restores={restores} spi_control={:?} spi_inten={:?} channel={:?} apbd={:?} gclk={:?} port_out={:?}",machine.cpu.pc,u32::from(machine.cpu.primask),machine.bus.read_u32(0x43000000),machine.bus.read_u8(0x43000016),machine.bus.read_u32(0x4100a040),machine.bus.read_u32(0x40000820),machine.bus.read_u32(0x40001d08),machine.bus.read_u32(0x41008090));
            if step == 4_000_000 || ![0, 0x100, 0x600d].contains(&v[0]) {
                for row in trace.rows.lock().unwrap().iter() {
                    println!(
                        "SAMD_REAL_SPI_TIMEOUT_FAILURE_ROW mode={mode} step={} kind={} value={}",
                        row.step, row.kind, row.value
                    );
                }
            }
        }
        if v[6] == 2 && !resumed {
            resumed = true;
            trace.record("async-resume", v[12]);
        }
        if v[14] == 1 && !blocked {
            blocked = true;
            trace.record("blocking-return", v[13]);
        }
        assert!(
            [0, 0x100, 0x600d].contains(&v[0]),
            "unexpected actual timeout status {:#x}",
            v[0]
        );
        if v[0] != 0x600d {
            continue;
        }
        assert_eq!(&v[..3], &[0x600d, 3, 1]);
        assert_eq!(v[3], swaps);
        assert!(swaps >= 8);
        assert_eq!(v[4], restores);
        assert_eq!(v[5], entries[0]);
        assert!(entries[0] >= 1);
        assert_eq!(entries, returns);
        assert_eq!(&entries[1..], &[2, 2]);
        assert_eq!(&v[6..10], &[3, 1, 79, 1]);
        assert_eq!(v[11], (-1006_i32) as u32);
        assert!(v[10] != 0 && v[10] % 8 == 0 && v[12] != 0 && v[13] != 0 && v[12] != v[13]);
        assert_eq!(v[14], 1);
        assert_eq!(&f[..6], &[4, 1, 3, 15, 0, 1]);
        assert_eq!(f[6], v[10]);
        assert_eq!(f[7], (-1006_i32) as u32);
        assert!(f[8] != 0 && f[8] != v[12] && f[9] != 0);
        assert_ne!(f[10] & u32::from(trace.gate_mask), 0);
        assert_eq!(f[11], f[10] & !u32::from(trace.gate_mask));
        assert_eq!(f[12], f[10]);
        assert_eq!(f[13], 0x2000c);
        assert_eq!(f[14], 0);
        assert_eq!(f[15], 0x200d00);
        assert!(f[16] > 0 && f[16] < 32);
        assert_eq!(f[16], f[17]);
        assert!(f[18] > 0 && f[18] < f[19] && f[19] < f[20]);
        assert_eq!(f[21], (-1006_i32) as u32);
        assert_eq!(f[22], 1);
        assert!(
            f[23] > f[20]
                && f[24].wrapping_sub(f[23]) >= 1499 * 120
                && f[26].wrapping_sub(f[25]) >= 1500
        );
        assert!(
            failure_seen
                && failure_returned
                && worker_seen
                && timeout_resumed
                && wait_seen
                && wait_started
                && wait_finished
        );
        assert!(machine.cpu.primask && exception == 0);
        assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
        assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
        let rows = trace.rows.lock().unwrap();
        for row in rows.iter() {
            println!(
                "SAMD_REAL_SPI_TIMEOUT_ROW mode={mode} step={} kind={} value={}",
                row.step, row.kind, row.value
            );
        }
        let split = recovery_start.expect("actual wait before recovery");
        timeout_ordering(&rows[..split], f[16] as u16, trace.gate_mask);
        ordering(&rows[split..]);
        assert_eq!(rows.len(), split + 28);
        assert!(rows.last().unwrap().step < step);
        for (kind, value) in [
            ("failure", f[6]),
            ("gate-worker", f[8]),
            ("timeout-resume", v[12]),
            ("wait-begin", f[23]),
            ("wait-end", f[24]),
        ] {
            assert_eq!(rows.iter().find(|r| r.kind == kind).unwrap().value, value);
        }
        println!("SAMD_REAL_SPI_TIMEOUT_RESULT mode={mode} source={source} elf_sha256={image} steps={step} status=0x600d startup=true vectors=true data_copy=true bss_clear=true witnesses={v:?} timeout={f:?} timeout_rows={split} irq_entries={entries:?} irq_returns={returns:?} actual_swaps={swaps} actual_restores={restores} final_mask=1 final_exception=0 runtime_admission=false");
        terminal = Some(step);
        break;
    }
    assert!(
        terminal.is_some(),
        "actual timeout guest exceeded unchanged bounded budget"
    );
}
