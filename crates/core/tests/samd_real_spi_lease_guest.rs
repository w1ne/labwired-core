// SPDX-License-Identifier: MIT
//! Actual blocking cancellation/result ownership; not general lifetime or native Arcade.
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
    cancelling: Mutex<bool>,
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
        if address == 0x43000028 {
            self.record("accept", u32::from(value));
        }
        if *self.cancelling.lock().unwrap() && (0x4100a040..=0x4100a043).contains(&address) {
            self.record(
                ["disable0", "disable1", "disable2", "disable3"][(address - 0x4100a040) as usize],
                u32::from(value),
            );
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
    assert_eq!(rows.len(), 28, "closed positive recovery census");
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
    assert!(rows.windows(2).all(|r| r[0].step <= r[1].step));
}
#[test]
fn manufactured_missing_spi_ordering_refuses() {
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

fn lease_ordering(rows: &[Row], remaining: u32, event: u32) {
    let roles = [
        "owner-call",
        "owner-ready",
        "lease-call",
        "enroll",
        "enroll-return",
        "select",
        "canceller-call",
        "canceller-ready",
        "cancel-call",
        "disable0",
        "disable1",
        "disable2",
        "disable3",
        "release",
        "notify-one",
        "notify-return",
        "cancel-return",
        "busy-call",
        "busy-return",
        "cleanup",
        "canceller-return",
        "canceller-return-fiber",
        "lease-return",
        "lease-return-fiber",
        "owner-notify",
        "owner-notify-return",
        "owner-return",
        "owner-return-fiber",
        "main-resume",
    ];
    let at = |kind| {
        let hits = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(hits.len(), 1, "unique lease {kind}");
        hits[0]
    };
    for role in roles {
        at(role);
    }
    let indices = |kind| {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let accept = indices("accept");
    let wire = indices("wire");
    assert!(remaining > 0 && remaining < 32 && event > 0 && event <= 65535);
    assert_eq!(accept.len(), (32 - remaining) as usize);
    assert!(wire.len() <= accept.len());
    assert_eq!(
        rows.len(),
        roles.len() + accept.len() + wire.len(),
        "closed lease census"
    );
    let mut payload = [0u32; 32];
    payload[..4].copy_from_slice(&[0x91, 0x82, 0x73, 0x64]);
    for (n, i) in accept.iter().enumerate() {
        assert_eq!(rows[*i].value, payload[n]);
        assert!(at("select") < *i && *i < at("disable0"));
    }
    for (n, i) in wire.iter().enumerate() {
        assert_eq!(rows[*i].value, payload[n]);
        assert!(accept[n] < *i && *i < at("release"));
    }
    for pair in roles.windows(2) {
        assert!(at(pair[0]) < at(pair[1]));
    }
    for (role, value) in [
        ("enroll", event),
        ("enroll-return", 0),
        ("select", 0),
        ("disable0", 0),
        ("disable1", 13),
        ("disable2", 32),
        ("disable3", 0),
        ("release", 0),
        ("notify-one", event),
        ("cancel-return", 0),
        ("busy-return", (-1006_i32) as u32),
        ("cleanup", remaining),
        ("lease-return", (-1014_i32) as u32),
        ("owner-notify", 1),
    ] {
        assert_eq!(rows[at(role)].value, value, "lease {role}");
    }
    for role in [
        "notify-return",
        "canceller-return",
        "owner-notify-return",
        "owner-return",
    ] {
        let v = rows[at(role)].value;
        assert!(v > 0 && v < 0x100000 && v.is_multiple_of(2));
    }
    for role in [
        "owner-call",
        "owner-ready",
        "lease-call",
        "canceller-call",
        "canceller-ready",
        "cancel-call",
        "busy-call",
        "canceller-return-fiber",
        "lease-return-fiber",
        "owner-return-fiber",
        "main-resume",
    ] {
        let v = rows[at(role)].value;
        assert!((0x20000000..0x2002e000).contains(&v) && v.is_multiple_of(8));
    }
    for role in [
        "owner-call",
        "lease-call",
        "lease-return-fiber",
        "owner-return-fiber",
    ] {
        assert_eq!(rows[at(role)].value, rows[at("owner-ready")].value);
    }
    for role in ["canceller-call", "canceller-return-fiber"] {
        assert_eq!(rows[at(role)].value, rows[at("canceller-ready")].value);
    }
    assert_eq!(rows[at("owner-ready")].value, rows[at("lease-call")].value);
    assert_eq!(
        rows[at("canceller-ready")].value,
        rows[at("cancel-call")].value
    );
    assert_eq!(rows[at("busy-call")].value, rows[at("cancel-call")].value);
    assert_ne!(
        rows[at("owner-ready")].value,
        rows[at("canceller-ready")].value
    );
    assert_ne!(rows[at("owner-ready")].value, rows[at("main-resume")].value);
    assert_ne!(
        rows[at("canceller-ready")].value,
        rows[at("main-resume")].value
    );
    assert!(rows.windows(2).all(|r| r[0].step <= r[1].step));
}
#[test]
fn manufactured_blocking_lease_epoch_refuses() {
    let raw = [
        ("owner-call", 0x20001000),
        ("owner-ready", 0x20001000),
        ("lease-call", 0x20001000),
        ("enroll", 7),
        ("enroll-return", 0),
        ("select", 0),
        ("accept", 0x91),
        ("wire", 0x91),
        ("accept", 0x82),
        ("canceller-call", 0x20002000),
        ("canceller-ready", 0x20002000),
        ("cancel-call", 0x20002000),
        ("disable0", 0),
        ("disable1", 13),
        ("disable2", 32),
        ("disable3", 0),
        ("release", 0),
        ("notify-one", 7),
        ("notify-return", 0xa00),
        ("cancel-return", 0),
        ("busy-call", 0x20002000),
        ("busy-return", (-1006_i32) as u32),
        ("cleanup", 30),
        ("canceller-return", 0xb00),
        ("canceller-return-fiber", 0x20002000),
        ("lease-return", (-1014_i32) as u32),
        ("lease-return-fiber", 0x20001000),
        ("owner-notify", 1),
        ("owner-notify-return", 0xc00),
        ("owner-return", 0xd00),
        ("owner-return-fiber", 0x20001000),
        ("main-resume", 0x20003000),
    ];
    let rows = raw
        .into_iter()
        .enumerate()
        .map(|(i, (kind, value))| Row {
            step: i + 1,
            kind,
            value,
        })
        .collect::<Vec<_>>();
    lease_ordering(&rows, 30, 7);
    // Cancellation may release CS before any accepted byte reaches the slave.
    // Removing an optional wire row is another valid prefix, not a refusal.
    let no_wire = rows
        .iter()
        .filter(|row| row.kind != "wire")
        .cloned()
        .collect::<Vec<_>>();
    lease_ordering(&no_wire, 30, 7);
    for i in 0..rows.len() {
        if rows[i].kind == "wire" {
            continue;
        }
        let mut bad = rows.clone();
        bad.remove(i);
        assert!(
            std::panic::catch_unwind(|| lease_ordering(&bad, 30, 7)).is_err(),
            "missing required lease observation {}",
            rows[i].kind
        );
    }
    for i in 0..rows.len() {
        let mut bad = rows.clone();
        bad[i].value ^= 255;
        assert!(std::panic::catch_unwind(|| lease_ordering(&bad, 30, 7)).is_err());
    }
    for (a, b) in [
        ("busy-return", "lease-return"),
        ("notify-one", "release"),
        ("enroll-return", "select"),
        ("owner-return", "main-resume"),
    ] {
        let mut bad = rows.clone();
        let a = bad.iter().position(|r| r.kind == a).unwrap();
        let b = bad.iter().position(|r| r.kind == b).unwrap();
        bad.swap(a, b);
        assert!(std::panic::catch_unwind(|| lease_ordering(&bad, 30, 7)).is_err());
    }
    for kind in [
        "accept", "wire", "release", "callback", "failure", "unknown",
    ] {
        let mut bad = rows.clone();
        bad.push(Row {
            step: rows.len() + 1,
            kind,
            value: 0,
        });
        assert!(std::panic::catch_unwind(|| lease_ordering(&bad, 30, 7)).is_err());
    }
}

#[test]
#[ignore = "actual source-bound CODAL blocking result lease requires dedicated hosted build and both scheduler profiles"]
fn actual_codal_spi_blocking_result_lease_and_recovery() {
    let folder = PathBuf::from(std::env::var("LABWIRED_SPI_FIXTURE_DIR").unwrap());
    let source = std::env::var("LABWIRED_SPI_SOURCE").unwrap();
    assert!(source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()));
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["source"], source);
    assert_eq!(provenance["caller_mode"], 14);
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
    let symbol = |name: &str| {
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
    let lease_names = [
        "stage",
        "owner_identity",
        "canceller_identity",
        "owner_entry_mask",
        "owner_entry_ipsr",
        "owner_return_mask",
        "owner_return_ipsr",
        "owner_result",
        "owner_returned",
        "cancel_result",
        "cancel_mask_before",
        "cancel_mask_after",
        "cancel_returned",
        "competing_result",
        "competing_returned",
        "failures",
        "cleanup_spi",
        "cleanup_inten",
        "cleanup_channel",
        "cleanup_cs",
        "saved_writeback",
        "remaining",
        "main_completed",
        "expected_error",
        "expected_notify",
    ]
    .map(|name| format!("runtime_spi_lease_{name}"));
    let lease_addresses = lease_names.each_ref().map(|name| symbol(name));
    let fiber_address = symbol("_ZN5codal12currentFiberE");
    let helper_addresses = ["owner", "transfer", "canceller", "cancel", "competing"]
        .map(|name| symbol(&format!("runtime_spi_lease_{name}")) as u32 & !1);
    let enroll_address = symbol("_ZN5codal19fiber_wake_on_eventEtt") as u32 & !1;
    let mut event_addresses = elf
        .syms
        .iter()
        .filter(|s| s.st_shndx != 0)
        .filter_map(|s| {
            let name = elf.strtab.get_at(s.st_name)?;
            (name.starts_with("_ZN5codal5EventC1EttN") || name.starts_with("_ZN5codal5EventC2EttN"))
                .then_some(s.st_value as u32 & !1)
        })
        .collect::<Vec<_>>();
    event_addresses.sort_unstable();
    event_addresses.dedup();
    assert_eq!(
        event_addresses.len(),
        1,
        "unique actual two-argument Event constructor"
    );
    let event_address = event_addresses[0];
    let bss = symbol("_bss_start")..symbol("_bss_end");
    let data = symbol("_data_start")..symbol("_data_end");
    let mut distinct = [
        addresses.to_vec(),
        lease_addresses.to_vec(),
        vec![fiber_address],
    ]
    .concat();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), names.len() + lease_names.len() + 1);
    assert!(addresses
        .iter()
        .chain(&lease_addresses)
        .chain([&fiber_address])
        .all(|a| bss.contains(a) && a % 4 == 0));
    assert!(data.start >= 0x20000000 && data.end <= bss.start && bss.end < 0x2002e000);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    chip.peripherals.push(serde_yaml::from_str("id: tx-dma\ntype: sam_dmac_sercom4_tx_byte\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\nclock: { controller: mclk, reg: AHBMASK, bit: 9 }\n").unwrap());
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let trace = Arc::new(Trace::default());
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
    let mut callback_seen = false;
    let mut callback_returned = false;
    let mut resumed = false;
    let mut blocked = false;
    let mut call_returns = [None; 5];
    let mut call_fibers = [0; 5];
    let mut call_seen = [false; 5];
    let mut call_exited = [false; 5];
    let mut owner_ready = false;
    let mut canceller_ready = false;
    let mut cleanup = false;
    let mut main_resumed = false;
    let mut enroll_return = None;
    let mut enrolled = false;
    let mut enroll_returned = false;
    let mut notify_event = 0;
    let mut notify_return = None;
    let mut notified = false;
    let mut notify_returned = false;
    let mut owner_notify_return = None;
    let mut owner_notified = false;
    let mut owner_notify_returned = false;
    let mut recovery_start = None;
    let mut terminal = None;
    let mut diagnostic_stage = 0;
    let mut diagnostic_lease_stage = 0;
    for step in 1..=4_000_000 {
        *trace.step.lock().unwrap() = step;
        let pc = machine.cpu.pc & !1;
        let fiber = if startup {
            machine.bus.read_u32(fiber_address).unwrap()
        } else {
            0
        };
        for i in 0..5 {
            if startup && pc == helper_addresses[i] {
                assert!(recovery_start.is_none() && !call_seen[i]);
                assert_eq!(machine.cpu.active_exception, 0);
                assert_eq!(machine.cpu.primask, i >= 3);
                assert!((0x20000000..0x2002e000).contains(&fiber) && fiber % 8 == 0);
                call_seen[i] = true;
                call_fibers[i] = fiber;
                call_returns[i] = Some(machine.cpu.lr & !1);
                if i == 3 {
                    *trace.cancelling.lock().unwrap() = true;
                }
                if i == 4 {
                    assert!(call_exited[3] && !call_exited[1]);
                }
                trace.record(
                    [
                        "owner-call",
                        "lease-call",
                        "canceller-call",
                        "cancel-call",
                        "busy-call",
                    ][i],
                    fiber,
                );
            }
            // Fiber entry functions share a trampoline LR: PC alone is not ownership.
            if startup
                && call_seen[i]
                && !call_exited[i]
                && Some(pc) == call_returns[i]
                && fiber == call_fibers[i]
            {
                call_exited[i] = true;
                assert_eq!(machine.cpu.active_exception, 0);
                assert_eq!(machine.cpu.primask, i >= 3);
                let value = match i {
                    0 => {
                        assert!(call_exited[1] && call_exited[2] && owner_notify_returned);
                        pc
                    }
                    1 => {
                        assert!(call_exited[2] && call_exited[4]);
                        assert_eq!(machine.cpu.r0, (-1014_i32) as u32);
                        machine.cpu.r0
                    }
                    2 => {
                        assert!(call_exited[3] && call_exited[4]);
                        pc
                    }
                    3 => {
                        assert!(notify_returned);
                        assert_eq!(machine.cpu.r0, 0);
                        *trace.cancelling.lock().unwrap() = false;
                        machine.cpu.r0
                    }
                    _ => {
                        assert!(!call_exited[1]);
                        assert_eq!(machine.cpu.r0, (-1006_i32) as u32);
                        machine.cpu.r0
                    }
                };
                trace.record(
                    [
                        "owner-return",
                        "lease-return",
                        "canceller-return",
                        "cancel-return",
                        "busy-return",
                    ][i],
                    value,
                );
                if i <= 2 {
                    trace.record(
                        [
                            "owner-return-fiber",
                            "lease-return-fiber",
                            "canceller-return-fiber",
                        ][i],
                        fiber,
                    );
                }
            }
        }
        if startup && call_seen[1] && !call_exited[1] && !enrolled && pc == enroll_address {
            assert_eq!(fiber, call_fibers[1]);
            assert!(machine.cpu.primask);
            assert_eq!(machine.cpu.active_exception, 0);
            assert_eq!(machine.cpu.r0, 1023);
            notify_event = machine.cpu.r1;
            assert!(notify_event > 0 && notify_event <= 65535);
            enrolled = true;
            enroll_return = Some(machine.cpu.lr & !1);
            trace.record("enroll", notify_event);
        }
        if startup
            && enrolled
            && !enroll_returned
            && Some(pc) == enroll_return
            && fiber == call_fibers[1]
        {
            assert!(machine.cpu.primask);
            assert_eq!(machine.cpu.active_exception, 0);
            assert_eq!(machine.cpu.r0, 0);
            enroll_returned = true;
            trace.record("enroll-return", machine.cpu.r0);
        }
        if startup && pc == event_address && call_seen[3] && !call_exited[3] {
            assert!(!notified && enroll_returned && fiber == call_fibers[2]);
            assert!(machine.cpu.primask);
            assert_eq!(machine.cpu.active_exception, 0);
            assert_eq!(
                (machine.cpu.r1, machine.cpu.r2, machine.cpu.r3),
                (1022, notify_event, 1)
            );
            notified = true;
            notify_return = Some(machine.cpu.lr & !1);
            trace.record("notify-one", machine.cpu.r2);
        }
        if startup
            && notified
            && !notify_returned
            && Some(pc) == notify_return
            && fiber == call_fibers[2]
        {
            assert!(machine.cpu.primask);
            assert_eq!(machine.cpu.active_exception, 0);
            notify_returned = true;
            trace.record("notify-return", pc);
        }
        if startup && pc == event_address && call_exited[1] && !call_exited[0] {
            assert!(!owner_notified && fiber == call_fibers[0]);
            assert!(!machine.cpu.primask);
            assert_eq!(machine.cpu.active_exception, 0);
            assert_eq!((machine.cpu.r1, machine.cpu.r2, machine.cpu.r3), (58, 1, 1));
            owner_notified = true;
            owner_notify_return = Some(machine.cpu.lr & !1);
            trace.record("owner-notify", machine.cpu.r2);
        }
        if startup
            && owner_notified
            && !owner_notify_returned
            && Some(pc) == owner_notify_return
            && fiber == call_fibers[0]
        {
            assert!(!machine.cpu.primask);
            assert_eq!(machine.cpu.active_exception, 0);
            owner_notify_returned = true;
            trace.record("owner-notify-return", pc);
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
            println!("SAMD_REAL_SPI_LEASE_STEP_ERROR step={step} source={source} elf_sha256={image} error={error:?} pc={:#x} lr={:#x} mask={} exception={}",machine.cpu.pc,machine.cpu.lr,u32::from(machine.cpu.primask),machine.cpu.active_exception);
            for row in trace.rows.lock().unwrap().iter() {
                println!(
                    "SAMD_REAL_SPI_LEASE_FAILURE_ROW step={} kind={} value={}",
                    row.step, row.kind, row.value
                );
            }
            panic!("actual blocking-lease step failed; original rows retained");
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
        let g = lease_addresses.map(|a| machine.bus.read_u32(a).unwrap());
        if g[1] != 0 && !owner_ready {
            assert!(call_seen[0]);
            owner_ready = true;
            trace.record("owner-ready", g[1]);
        }
        if g[2] != 0 && !canceller_ready {
            assert!(call_seen[2]);
            canceller_ready = true;
            trace.record("canceller-ready", g[2]);
        }
        if g[0] == 3 && !cleanup {
            assert!(call_exited[4] && machine.cpu.primask);
            cleanup = true;
            trace.record("cleanup", g[21]);
        }
        if g[0] == 5 && !main_resumed {
            assert!(call_exited.into_iter().all(|v| v) && exception == 0 && !machine.cpu.primask);
            assert_eq!(machine.bus.read_u32(fiber_address).unwrap(), v[12]);
            main_resumed = true;
            trace.record("main-resume", v[12]);
        }
        if v[6] == 1 && recovery_start.is_none() {
            assert!(
                main_resumed
                    && g[0] == 5
                    && enroll_returned
                    && notify_returned
                    && owner_notify_returned
            );
            assert_eq!(&entries[1..], &[0, 0]);
            assert_eq!(entries, returns);
            recovery_start = Some(trace.rows.lock().unwrap().len());
        }
        if v[6] != diagnostic_stage
            || g[0] != diagnostic_lease_stage
            || step == 4_000_000
            || ![0, 0x100, 0x600d].contains(&v[0])
        {
            diagnostic_stage = v[6];
            diagnostic_lease_stage = g[0];
            // A refused controller can reject diagnostic reads too. Preserve
            // the actual Err rather than panicking before state/rows are logged.
            let wb = machine.bus.read_u32(0x4100a038);
            let wb_count = wb.as_ref().ok().and_then(|wb| {
                (bss.contains(&u64::from(*wb)) && wb % 16 == 0)
                    .then(|| machine.bus.read_u16(u64::from(wb + 2)))
            });
            println!("SAMD_REAL_SPI_LEASE_DIAGNOSTIC step={step} source={source} elf_sha256={image} lease_witnesses={g:?} notify_event={notify_event} pc={:#x} lr={:#x} sp={:#x} mask={} exception={exception} witnesses={v:?} irq_entries={entries:?} irq_returns={returns:?} swaps={swaps} restores={restores} dma_control={:?} dma_channel={:?} dma_flags={:?} wb={wb:?} wb_count={wb_count:?} spi_control={:?} spi_flags={:?} ahb={:?} apbd={:?} gclk={:?} port_out={:?}", machine.cpu.pc, machine.cpu.lr, machine.cpu.sp, u32::from(machine.cpu.primask), machine.bus.read_u32(0x4100a000), machine.bus.read_u32(0x4100a040), machine.bus.read_u8(0x4100a04e), machine.bus.read_u32(0x43000000), machine.bus.read_u8(0x43000018), machine.bus.read_u32(0x40000810), machine.bus.read_u32(0x40000820), machine.bus.read_u32(0x40001d08), machine.bus.read_u32(0x41008090));
            if step == 4_000_000 || ![0, 0x100, 0x600d].contains(&v[0]) {
                for row in trace.rows.lock().unwrap().iter() {
                    println!(
                        "SAMD_REAL_SPI_LEASE_FAILURE_ROW step={} kind={} value={}",
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
            "unexpected actual SPI status {:#x}",
            v[0]
        );
        if v[0] != 0x600d {
            continue;
        }
        assert_eq!(&v[..3], &[0x600d, 3, 1]);
        assert_eq!(v[3], swaps);
        assert!(swaps >= 3);
        assert_eq!(v[4], restores);
        assert_eq!(v[5], entries[0]);
        assert_eq!(entries, returns);
        assert_eq!(&entries[1..], &[2, 2]);
        assert_eq!(&v[6..10], &[3, 1, 79, 1]);
        assert_eq!(v[11], (-1006_i32) as u32);
        assert!(v[10] != 0 && v[10] % 8 == 0 && v[12] != 0 && v[13] != 0 && v[12] != v[13]);
        assert_eq!(v[14], 1);
        assert!(machine.cpu.primask && exception == 0);
        assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
        assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
        assert_eq!(g[0], 5);
        assert_eq!(&g[3..9], &[0, 0, 0, 0, (-1014_i32) as u32, 1]);
        assert_eq!(&g[9..16], &[0, 0, 0, 1, (-1006_i32) as u32, 1, 0]);
        assert_eq!(&g[16..20], &[0x2000c, 0, 0x200d00, 1]);
        assert!(bss.contains(&u64::from(g[20])) && g[20] % 16 == 0 && g[21] > 0 && g[21] < 32);
        assert_eq!(&g[22..], &[1, (-1014_i32) as u32, 1022]);
        assert_eq!(g[1], call_fibers[0]);
        assert_eq!(g[1], call_fibers[1]);
        assert_eq!(g[2], call_fibers[2]);
        assert_eq!(g[2], call_fibers[3]);
        assert_eq!(g[2], call_fibers[4]);
        assert!(g[1] != g[2] && g[1] != v[12] && g[2] != v[12]);
        assert!(
            call_exited.into_iter().all(|v| v)
                && owner_ready
                && canceller_ready
                && cleanup
                && main_resumed
        );
        assert!(enroll_returned && notify_returned && owner_notify_returned && swaps >= 7);
        let rows = trace.rows.lock().unwrap();
        let boundary = recovery_start.unwrap();
        let (prefix, recovery) = rows.split_at(boundary);
        for row in prefix {
            println!(
                "SAMD_REAL_SPI_LEASE_PREFIX_ROW step={} kind={} value={}",
                row.step, row.kind, row.value
            );
        }
        for row in recovery {
            println!(
                "SAMD_REAL_SPI_LEASE_ROW step={} kind={} value={}",
                row.step, row.kind, row.value
            );
        }
        lease_ordering(prefix, g[21], notify_event);
        for (role, value) in [
            ("owner-ready", g[1]),
            ("canceller-ready", g[2]),
            ("main-resume", v[12]),
        ] {
            assert_eq!(prefix.iter().find(|r| r.kind == role).unwrap().value, value);
        }
        ordering(recovery);
        println!("SAMD_REAL_SPI_LEASE_RESULT source={source} elf_sha256={image} steps={step} lease_witnesses={g:?} notify_event={notify_event} prefix_rows={boundary} status=0x600d startup=true vectors=true data_copy=true bss_clear=true witnesses={v:?} irq_entries={entries:?} irq_returns={returns:?} actual_swaps={swaps} actual_restores={restores} final_mask=1 final_exception=0 runtime_admission=false");
        terminal = Some(step);
        break;
    }
    assert!(
        terminal.is_some(),
        "actual SPI guest exceeded unchanged bounded budget"
    );
}
