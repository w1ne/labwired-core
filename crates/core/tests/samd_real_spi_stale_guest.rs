// SPDX-License-Identifier: MIT
//! Actual software-pended idle IRQ dispatch, not naturally stale hardware interrupts.
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
        if (0xe000e200..=0xe000e207).contains(&address) {
            let role = [
                "pend0-0", "pend0-1", "pend0-2", "pend0-3", "pend1-0", "pend1-1", "pend1-2",
                "pend1-3",
            ][(address - 0xe000e200) as usize];
            self.record(role, u32::from(value));
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
    assert_eq!(rows.len(), 28, "closed unchanged recovery census");
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

fn stale_ordering(rows: &[Row]) {
    let roles = [
        "select",
        "dma",
        "txc",
        "release",
        "prime-callback",
        "prime-return",
        "prime-resume",
        "stale-call",
        "stale-armed",
        "pend0-0",
        "pend0-1",
        "pend0-2",
        "pend0-3",
        "pend1-0",
        "pend1-1",
        "pend1-2",
        "pend1-3",
        "stale-dma",
        "stale-dma-return",
        "stale-spi",
        "stale-spi-return",
        "stale-return",
    ];
    let at = |kind| {
        let hits = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(hits.len(), 1, "unique prefix {kind}");
        hits[0]
    };
    for role in roles {
        at(role);
    }
    assert_eq!(rows.len(), roles.len() + 8, "closed raw prefix census");
    let indices = |kind| {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let accept = indices("accept");
    let wire = indices("wire");
    assert_eq!(accept.len(), 4);
    assert_eq!(wire.len(), 4);
    assert_eq!(
        accept.iter().map(|i| rows[*i].value).collect::<Vec<_>>(),
        [0x12, 0x34, 0x56, 0x78]
    );
    assert_eq!(
        wire.iter().map(|i| rows[*i].value).collect::<Vec<_>>(),
        [0x12, 0x34, 0x56, 0x78]
    );
    assert!(at("select") < accept[0] && accept[3] < at("dma"));
    assert!(at("dma") < wire[3] && wire[3] < at("txc") && at("txc") < at("release"));
    for i in 0..4 {
        assert!(accept[i] < wire[i]);
    }
    for pair in [
        "release",
        "prime-callback",
        "prime-return",
        "prime-resume",
        "stale-call",
        "stale-armed",
        "pend0-0",
        "pend0-1",
        "pend0-2",
        "pend0-3",
        "pend1-0",
        "pend1-1",
        "pend1-2",
        "pend1-3",
    ]
    .windows(2)
    {
        assert!(at(pair[0]) < at(pair[1]));
    }
    for (entry, returned) in [
        ("stale-dma", "stale-dma-return"),
        ("stale-spi", "stale-spi-return"),
    ] {
        assert!(
            at("pend1-3") < at(entry)
                && at(entry) < at(returned)
                && at(returned) < at("stale-return")
        );
    }
    for (role, value) in [
        ("select", 0),
        ("dma", 0),
        ("txc", 0),
        ("release", 0),
        ("stale-armed", 1),
        ("pend0-0", 0),
        ("pend0-1", 0),
        ("pend0-2", 0),
        ("pend0-3", 128),
        ("pend1-0", 0),
        ("pend1-1", 0),
        ("pend1-2", 0),
        ("pend1-3", 128),
        ("stale-dma", 0),
        ("stale-dma-return", 47),
        ("stale-spi-return", 79),
    ] {
        assert_eq!(rows[at(role)].value, value, "raw prefix {role}");
    }
    assert_ne!(
        rows[at("stale-spi")].value & 2,
        0,
        "hardware TXC remains sticky but delivery is settled"
    );
    for role in ["prime-callback", "prime-resume", "stale-call"] {
        assert!((0x20000000..0x2002e000).contains(&rows[at(role)].value));
        assert_eq!(rows[at(role)].value % 8, 0);
    }
    assert_eq!(rows[at("prime-resume")].value, rows[at("stale-call")].value);
    for role in ["prime-return", "stale-return"] {
        assert!(rows[at(role)].value > 0 && rows[at(role)].value < 0x100000);
        assert_eq!(rows[at(role)].value % 2, 0);
    }
    assert!(rows.windows(2).all(|r| r[0].step <= r[1].step));
}
#[test]
fn manufactured_stale_dispatch_epoch_refuses() {
    let mut raw = vec![("select", 0)];
    for (i, byte) in [0x12, 0x34, 0x56, 0x78].into_iter().enumerate() {
        raw.push(("accept", byte));
        if i == 3 {
            raw.push(("dma", 0));
        }
        raw.push(("wire", byte));
    }
    raw.extend([
        ("txc", 0),
        ("release", 0),
        ("prime-callback", 0x20002000),
        ("prime-return", 0x800),
        ("prime-resume", 0x20001000),
        ("stale-call", 0x20001000),
        ("stale-armed", 1),
        ("pend0-0", 0),
        ("pend0-1", 0),
        ("pend0-2", 0),
        ("pend0-3", 128),
        ("pend1-0", 0),
        ("pend1-1", 0),
        ("pend1-2", 0),
        ("pend1-3", 128),
        ("stale-dma", 0),
        ("stale-dma-return", 47),
        ("stale-spi", 3),
        ("stale-spi-return", 79),
        ("stale-return", 0x900),
    ]);
    let rows = raw
        .into_iter()
        .enumerate()
        .map(|(i, (kind, value))| Row {
            step: i + 1,
            kind,
            value,
        })
        .collect::<Vec<_>>();
    stale_ordering(&rows);
    for index in 0..rows.len() {
        let mut bad = rows.clone();
        bad.remove(index);
        assert!(std::panic::catch_unwind(|| stale_ordering(&bad)).is_err());
    }
    for role in [
        "accept",
        "wire",
        "pend0-0",
        "pend0-3",
        "pend1-3",
        "stale-dma",
        "stale-dma-return",
        "stale-spi-return",
        "prime-callback",
        "stale-call",
    ] {
        let mut bad = rows.clone();
        bad.iter_mut().find(|r| r.kind == role).unwrap().value ^= 255;
        assert!(std::panic::catch_unwind(|| stale_ordering(&bad)).is_err());
    }
    for (a, b) in [
        ("stale-dma", "pend1-3"),
        ("stale-return", "stale-spi-return"),
        ("prime-resume", "prime-return"),
        ("stale-call", "prime-resume"),
    ] {
        let mut bad = rows.clone();
        let a = bad.iter().position(|r| r.kind == a).unwrap();
        let b = bad.iter().position(|r| r.kind == b).unwrap();
        bad.swap(a, b);
        assert!(std::panic::catch_unwind(|| stale_ordering(&bad)).is_err());
    }
    for role in [
        "wire",
        "accept",
        "release",
        "prime-callback",
        "stale-dma",
        "unknown",
    ] {
        let mut bad = rows.clone();
        bad.push(Row {
            step: rows.last().unwrap().step + 1,
            kind: role,
            value: 0,
        });
        assert!(std::panic::catch_unwind(|| stale_ordering(&bad)).is_err());
    }
}

#[test]
#[ignore = "actual source-bound CODAL software-pended stale IRQ caller requires dedicated hosted build and both scheduler profiles"]
fn actual_codal_spi_stale_dispatch_and_recovery() {
    let folder = PathBuf::from(std::env::var("LABWIRED_SPI_FIXTURE_DIR").unwrap());
    let source = std::env::var("LABWIRED_SPI_SOURCE").unwrap();
    assert!(source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()));
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["source"], source);
    assert_eq!(provenance["caller_mode"], 13);
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
    let stale_names = [
        "stage",
        "prime_callbacks",
        "prime_ipsr",
        "prime_cs",
        "prime_argument",
        "prime_reentry",
        "prime_fiber",
        "failures",
        "dispatch_fiber",
        "mask_before",
        "mask_after",
        "pending0_before",
        "pending1_before",
        "pending0_armed",
        "pending1_armed",
        "pending0_after",
        "pending1_after",
        "dma_flags_before",
        "spi_flags_before",
        "inten_before",
        "control_before",
        "dma_flags_after",
        "spi_flags_after",
        "inten_after",
        "control_after",
    ]
    .map(|name| format!("runtime_spi_stale_{name}"));
    let stale_addresses = stale_names.each_ref().map(|name| symbol(name));
    let bss = symbol("_bss_start")..symbol("_bss_end");
    let data = symbol("_data_start")..symbol("_data_end");
    let mut distinct = [addresses.to_vec(), stale_addresses.to_vec()].concat();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), names.len() + stale_names.len());
    assert!(addresses
        .iter()
        .chain(&stale_addresses)
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
    let mut prime_return = None;
    let mut prime_seen = false;
    let mut prime_returned = false;
    let mut prime_resumed = false;
    let mut dispatch_return = None;
    let mut dispatch_seen = false;
    let mut dispatch_returned = false;
    let mut software_dispatch = false;
    let mut armed_seen = false;
    let mut recovery_start = None;
    let mut stale_entries = [0; 2];
    let mut stale_returns = [0; 2];
    let mut terminal = None;
    let mut diagnostic_stage = 0;
    let mut diagnostic_stale_stage = 0;
    for step in 1..=4_000_000 {
        *trace.step.lock().unwrap() = step;
        let pc = machine.cpu.pc & !1;
        if startup && pc == symbol("runtime_spi_stale_done") as u32 & !1 {
            assert!(!prime_seen && recovery_start.is_none());
            prime_seen = true;
            assert_eq!(machine.cpu.active_exception, 79);
            prime_return = Some(machine.cpu.lr & !1);
            trace.record("prime-callback", machine.cpu.r0);
        }
        if startup && prime_seen && !prime_returned && Some(pc) == prime_return {
            prime_returned = true;
            assert_eq!(machine.cpu.active_exception, 79);
            trace.record("prime-return", pc);
        }
        if startup && pc == symbol("runtime_spi_stale_dispatch") as u32 & !1 {
            assert!(!dispatch_seen && prime_resumed && prime_returned);
            assert_eq!(machine.cpu.active_exception, 0);
            assert!(!machine.cpu.primask);
            dispatch_seen = true;
            software_dispatch = true;
            dispatch_return = Some(machine.cpu.lr & !1);
            trace.record("stale-call", machine.bus.read_u32(addresses[12]).unwrap());
        }
        if startup && dispatch_seen && !dispatch_returned && Some(pc) == dispatch_return {
            dispatch_returned = true;
            software_dispatch = false;
            assert_eq!(machine.cpu.active_exception, 0);
            assert!(!machine.cpu.primask);
            assert_eq!(stale_entries, [1, 1]);
            assert_eq!(stale_returns, [1, 1]);
            trace.record("stale-return", pc);
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
            println!("SAMD_REAL_SPI_STALE_STEP_ERROR step={step} source={source} elf_sha256={image} error={error:?} pc={:#x} lr={:#x} sp={:#x} mask={} exception={} irq_entries={entries:?} irq_returns={returns:?}", machine.cpu.pc, machine.cpu.lr, machine.cpu.sp, u32::from(machine.cpu.primask), machine.cpu.active_exception);
            for row in trace.rows.lock().unwrap().iter() {
                println!(
                    "SAMD_REAL_SPI_STALE_FAILURE_ROW step={} kind={} value={}",
                    row.step, row.kind, row.value
                );
            }
            panic!("actual stale-dispatch guest step failed; original rows retained");
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
                if software_dispatch && [47, 79].contains(&previous) {
                    let index = usize::from(previous == 79);
                    stale_returns[index] += 1;
                    trace.record(
                        if previous == 47 {
                            "stale-dma-return"
                        } else {
                            "stale-spi-return"
                        },
                        previous,
                    );
                }
                returns[[15, 47, 79].iter().position(|x| *x == previous).unwrap()] += 1;
            }
            if exception != 0 {
                entries[[15, 47, 79].iter().position(|x| *x == exception).unwrap()] += 1;
                if software_dispatch {
                    assert!(
                        [47, 79].contains(&exception),
                        "no unrelated software epoch exception"
                    );
                    let index = usize::from(exception == 79);
                    stale_entries[index] += 1;
                    let flags = machine
                        .bus
                        .read_u8(if exception == 47 {
                            0x4100a04e
                        } else {
                            0x43000018
                        })
                        .unwrap();
                    if exception == 47 {
                        assert_eq!(flags, 0);
                    } else {
                        assert_ne!(flags & 2, 0);
                    }
                    assert_eq!(machine.bus.read_u8(0x43000016).unwrap(), 0);
                    assert_ne!(machine.bus.read_u32(0x41008090).unwrap() & (1 << 7), 0);
                    assert!(armed_seen && recovery_start.is_none());
                    trace.record(
                        if exception == 47 {
                            "stale-dma"
                        } else {
                            "stale-spi"
                        },
                        u32::from(flags),
                    );
                } else if exception == 47 {
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
        let g = stale_addresses.map(|a| machine.bus.read_u32(a).unwrap());
        if g[0] == 2 && !prime_resumed {
            assert!(prime_returned && exception == 0 && !machine.cpu.primask);
            prime_resumed = true;
            trace.record("prime-resume", v[12]);
        }
        if g[0] == 3 && !armed_seen {
            assert!(dispatch_seen && software_dispatch && machine.cpu.primask);
            armed_seen = true;
            trace.record("stale-armed", u32::from(machine.cpu.primask));
        }
        if v[6] == 1 && recovery_start.is_none() {
            assert!(dispatch_returned && g[0] == 4);
            assert_eq!(entries, returns);
            assert_eq!(&entries[1..], &[2, 2]);
            let rows = trace.rows.lock().unwrap();
            recovery_start = Some(rows.len());
        }
        if v[6] != diagnostic_stage
            || g[0] != diagnostic_stale_stage
            || step == 4_000_000
            || ![0, 0x100, 0x600d].contains(&v[0])
        {
            diagnostic_stage = v[6];
            diagnostic_stale_stage = g[0];
            // A refused controller can reject diagnostic reads too. Preserve
            // the actual Err rather than panicking before state/rows are logged.
            let wb = machine.bus.read_u32(0x4100a038);
            let wb_count = wb.as_ref().ok().and_then(|wb| {
                (bss.contains(&u64::from(*wb)) && wb % 16 == 0)
                    .then(|| machine.bus.read_u16(u64::from(wb + 2)))
            });
            println!("SAMD_REAL_SPI_STALE_DIAGNOSTIC step={step} source={source} elf_sha256={image} stale_witnesses={g:?} pc={:#x} lr={:#x} sp={:#x} mask={} exception={exception} witnesses={v:?} irq_entries={entries:?} irq_returns={returns:?} swaps={swaps} restores={restores} dma_control={:?} dma_channel={:?} dma_flags={:?} wb={wb:?} wb_count={wb_count:?} spi_control={:?} spi_flags={:?} ahb={:?} apbd={:?} gclk={:?} port_out={:?}", machine.cpu.pc, machine.cpu.lr, machine.cpu.sp, u32::from(machine.cpu.primask), machine.bus.read_u32(0x4100a000), machine.bus.read_u32(0x4100a040), machine.bus.read_u8(0x4100a04e), machine.bus.read_u32(0x43000000), machine.bus.read_u8(0x43000018), machine.bus.read_u32(0x40000810), machine.bus.read_u32(0x40000820), machine.bus.read_u32(0x40001d08), machine.bus.read_u32(0x41008090));
            if step == 4_000_000 || ![0, 0x100, 0x600d].contains(&v[0]) {
                for row in trace.rows.lock().unwrap().iter() {
                    println!(
                        "SAMD_REAL_SPI_STALE_FAILURE_ROW step={} kind={} value={}",
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
        assert_eq!(&entries[1..], &[4, 4]);
        assert_eq!(&v[6..10], &[3, 1, 79, 1]);
        assert_eq!(v[11], (-1006_i32) as u32);
        assert!(v[10] != 0 && v[10] % 8 == 0 && v[12] != 0 && v[13] != 0 && v[12] != v[13]);
        assert_eq!(v[14], 1);
        assert!(machine.cpu.primask && exception == 0);
        assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
        assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
        assert_eq!(&g[..6], &[4, 1, 79, 1, v[10], (-1006_i32) as u32]);
        assert!(g[6] >= 0x20000000 && g[6] < 0x2002e000 && g[6] % 8 == 0);
        assert_eq!(&g[7..11], &[0, v[12], 0, 0]);
        for index in [11, 12, 15, 16] {
            assert_eq!(g[index] & 0x80000000, 0);
        }
        for index in [13, 14] {
            assert_ne!(g[index] & 0x80000000, 0);
        }
        assert_eq!(g[17], 0);
        assert_ne!(g[18] & 2, 0);
        assert_eq!(g[19], 0);
        assert_eq!(g[20], 0x2000e);
        assert_eq!(&g[17..21], &g[21..25]);
        assert!(
            prime_seen
                && prime_returned
                && prime_resumed
                && dispatch_seen
                && dispatch_returned
                && armed_seen
        );
        assert_eq!(stale_entries, [1, 1]);
        assert_eq!(stale_returns, [1, 1]);
        assert!(swaps >= 5);
        let rows = trace.rows.lock().unwrap();
        let boundary = recovery_start.unwrap();
        let (prefix, recovery) = rows.split_at(boundary);
        for row in prefix {
            println!(
                "SAMD_REAL_SPI_STALE_PREFIX_ROW step={} kind={} value={}",
                row.step, row.kind, row.value
            );
        }
        for row in recovery {
            println!(
                "SAMD_REAL_SPI_STALE_ROW step={} kind={} value={}",
                row.step, row.kind, row.value
            );
        }
        stale_ordering(prefix);
        for (role, expected) in [
            ("prime-callback", g[4]),
            ("prime-resume", v[12]),
            ("stale-call", g[8]),
        ] {
            assert_eq!(
                prefix.iter().find(|row| row.kind == role).unwrap().value,
                expected
            );
        }
        ordering(recovery);
        println!("SAMD_REAL_SPI_STALE_RESULT source={source} elf_sha256={image} steps={step} stale_witnesses={g:?} stale_irq_entries={stale_entries:?} stale_irq_returns={stale_returns:?} prefix_rows={boundary} status=0x600d startup=true vectors=true data_copy=true bss_clear=true witnesses={v:?} irq_entries={entries:?} irq_returns={returns:?} actual_swaps={swaps} actual_restores={restores} final_mask=1 final_exception=0 runtime_admission=false");
        terminal = Some(step);
        break;
    }
    assert!(
        terminal.is_some(),
        "actual SPI guest exceeded unchanged bounded budget"
    );
}
