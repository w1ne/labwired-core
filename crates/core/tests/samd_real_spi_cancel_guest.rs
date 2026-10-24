// SPDX-License-Identifier: MIT
//! Actual CODAL cancellation/recovery; not complete TX/native Arcade admission.
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
    assert!(rows.windows(2).all(|r| r[0].step <= r[1].step));
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

fn cancellation_ordering(rows: &[Row], expected_rows: usize, remaining: u16) {
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
    let worker = indices("cancel-worker");
    let failure = indices("failure");
    let returned = indices("failure-return");
    let cancelled = indices("cancel-return");
    let resumed = indices("cancel-resume");
    assert_eq!(
        [
            select.len(),
            release.len(),
            worker.len(),
            failure.len(),
            returned.len(),
            cancelled.len(),
            resumed.len()
        ],
        [1; 7]
    );
    assert_eq!(rows.len(), expected_rows);
    assert_eq!(rows.len(), 7 + accept.len() + wire.len());
    assert!(remaining > 0 && remaining < 32);
    assert_eq!(accept.len(), 32 - usize::from(remaining));
    assert!(!accept.is_empty() && accept.len() < 32 && wire.len() <= accept.len());
    let mut bytes = [0; 32];
    bytes[..4].copy_from_slice(&[0x91, 0x82, 0x73, 0x64]);
    for (beat, i) in accept.iter().enumerate() {
        assert_eq!(rows[*i].value, bytes[beat]);
        assert!(select[0] < *i && *i < release[0]);
    }
    for (beat, i) in wire.iter().enumerate() {
        assert_eq!(rows[*i].value, bytes[beat]);
        assert!(accept[beat] < *i && *i < release[0]);
    }
    assert!(
        worker[0] < release[0]
            && release[0] < failure[0]
            && failure[0] < returned[0]
            && returned[0] < cancelled[0]
            && cancelled[0] < resumed[0]
    );
    assert!(rows.windows(2).all(|r| r[0].step <= r[1].step));
}

#[test]
fn manufactured_missing_or_premature_cancellation_refuses() {
    let mut rows = [
        ("select", 0),
        ("accept", 0x91),
        ("accept", 0x82),
        ("wire", 0x91),
        ("cancel-worker", 0x20002100),
        ("release", 0),
        ("failure", 0x20001000),
        ("failure-return", 0x800),
        ("cancel-return", 0x20002100),
        ("cancel-resume", 0x20002000),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (kind, value))| Row {
        step: i + 1,
        kind,
        value,
    })
    .collect::<Vec<_>>();
    cancellation_ordering(&rows, 10, 30);
    // Losing any causal row or sending the wrong partial byte must refuse.
    for i in 0..rows.len() {
        let mut bad = rows.clone();
        bad.remove(i);
        assert!(std::panic::catch_unwind(|| cancellation_ordering(&bad, 10, 30)).is_err());
    }
    rows[3].value = 0xff;
    assert!(std::panic::catch_unwind(|| cancellation_ordering(&rows, 10, 30)).is_err());
    rows[3].value = 0x91;
    rows.swap(5, 6);
    assert!(std::panic::catch_unwind(|| cancellation_ordering(&rows, 10, 30)).is_err());
}

#[test]
#[ignore = "actual source-bound CODAL cancellation caller requires dedicated hosted build and both scheduler profiles"]
fn actual_codal_spi_cancellation_and_recovery() {
    let folder = PathBuf::from(std::env::var("LABWIRED_SPI_FIXTURE_DIR").unwrap());
    let source = std::env::var("LABWIRED_SPI_SOURCE").unwrap();
    assert!(source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()));
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["source"], source);
    assert_eq!(provenance["caller_mode"], 9);
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
    let cancel_names = [
        "runtime_spi_cancel_stage",
        "runtime_spi_failures",
        "runtime_spi_failure_result",
        "runtime_spi_failure_ipsr",
        "runtime_spi_failure_mask",
        "runtime_spi_failure_cs",
        "runtime_spi_failure_argument",
        "runtime_spi_failure_reentry",
        "runtime_spi_cancel_worker",
        "runtime_spi_failure_fiber",
        "runtime_spi_cancel_returned",
        "runtime_spi_cancel_busy",
    ];
    let cancel_addresses = cancel_names.map(symbol);
    let bss = symbol("_bss_start")..symbol("_bss_end");
    let data = symbol("_data_start")..symbol("_data_end");
    let mut distinct = [addresses.to_vec(), cancel_addresses.to_vec()].concat();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), names.len() + cancel_names.len());
    assert!(addresses
        .iter()
        .chain(&cancel_addresses)
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
    let mut terminal = None;
    let mut diagnostic_stage = 0;
    let mut diagnostic_cancel_stage = 0;
    let mut failure_return = None;
    let mut failure_seen = false;
    let mut failure_returned = false;
    let mut cancel_worker_seen = false;
    let mut cancel_returned = false;
    let mut recovery_start = None;
    let mut cancel_remaining = None;
    for step in 1..=4_000_000 {
        *trace.step.lock().unwrap() = step;
        let pc = machine.cpu.pc & !1;
        if startup && pc == symbol("runtime_spi_failed") as u32 & !1 {
            assert!(!failure_seen);
            failure_seen = true;
            assert_eq!(machine.cpu.active_exception, 0);
            assert!(machine.cpu.primask);
            assert_eq!(machine.cpu.r1, 2); // Actual cancelled enum argument.
            failure_return = Some(machine.cpu.lr & !1);
            trace.record("failure", machine.cpu.r0);
        }
        if startup && failure_seen && !failure_returned && Some(pc) == failure_return {
            failure_returned = true;
            assert_eq!(machine.cpu.active_exception, 0);
            assert!(machine.cpu.primask);
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
            println!("SAMD_REAL_SPI_CANCEL_STEP_ERROR step={step} source={source} elf_sha256={image} pc={:#x} error={error:?}", machine.cpu.pc);
            for row in trace.rows.lock().unwrap().iter() {
                println!(
                    "SAMD_REAL_SPI_CANCEL_FAILURE_ROW step={} kind={} value={}",
                    row.step, row.kind, row.value
                );
            }
            panic!("actual SPI guest must not fault: {error:?}");
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
        let f = cancel_addresses.map(|a| machine.bus.read_u32(a).unwrap());
        if f[8] != 0 && !cancel_worker_seen {
            cancel_worker_seen = true;
            trace.record("cancel-worker", f[8]);
        }
        if f[10] == 1 && !cancel_returned {
            cancel_returned = true;
            let wb = machine.bus.read_u32(0x4100a038).unwrap();
            assert!(bss.contains(&u64::from(wb)) && wb % 16 == 0);
            cancel_remaining = Some(machine.bus.read_u16(u64::from(wb + 2)).unwrap());
            trace.record("cancel-return", f[8]);
        }
        if f[0] == 2 && recovery_start.is_none() {
            trace.record("cancel-resume", v[12]);
            recovery_start = Some(trace.rows.lock().unwrap().len());
        }
        if v[6] != diagnostic_stage
            || f[0] != diagnostic_cancel_stage
            || step == 4_000_000
            || ![0, 0x100, 0x600d].contains(&v[0])
        {
            diagnostic_stage = v[6];
            diagnostic_cancel_stage = f[0];
            println!("SAMD_REAL_SPI_CANCEL_FAILURE_WITNESSES step={step} source={source} elf_sha256={image} cancellation={f:?}");
            // A refused controller can reject diagnostic reads too. Preserve
            // the actual Err rather than panicking before state/rows are logged.
            let wb = machine.bus.read_u32(0x4100a038);
            let wb_count = wb.as_ref().ok().and_then(|wb| {
                (bss.contains(&u64::from(*wb)) && wb % 16 == 0)
                    .then(|| machine.bus.read_u16(u64::from(wb + 2)))
            });
            println!("SAMD_REAL_SPI_CANCEL_DIAGNOSTIC step={step} source={source} elf_sha256={image} pc={:#x} lr={:#x} sp={:#x} mask={} exception={exception} witnesses={v:?} irq_entries={entries:?} irq_returns={returns:?} swaps={swaps} restores={restores} dma_control={:?} dma_channel={:?} dma_flags={:?} wb={wb:?} wb_count={wb_count:?} spi_control={:?} spi_flags={:?} ahb={:?} apbd={:?} gclk={:?} port_out={:?}", machine.cpu.pc, machine.cpu.lr, machine.cpu.sp, u32::from(machine.cpu.primask), machine.bus.read_u32(0x4100a000), machine.bus.read_u32(0x4100a040), machine.bus.read_u8(0x4100a04e), machine.bus.read_u32(0x43000000), machine.bus.read_u8(0x43000018), machine.bus.read_u32(0x40000810), machine.bus.read_u32(0x40000820), machine.bus.read_u32(0x40001d08), machine.bus.read_u32(0x41008090));
            if step == 4_000_000 || ![0, 0x100, 0x600d].contains(&v[0]) {
                for row in trace.rows.lock().unwrap().iter() {
                    println!(
                        "SAMD_REAL_SPI_CANCEL_FAILURE_ROW step={} kind={} value={}",
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
        assert!(swaps >= 8);
        assert_eq!(v[4], restores);
        assert_eq!(v[5], entries[0]);
        assert_eq!(entries, returns);
        assert_eq!(&entries[1..], &[2, 2]);
        assert_eq!(&v[6..10], &[3, 1, 79, 1]);
        assert_eq!(v[11], (-1006_i32) as u32);
        assert!(v[10] != 0 && v[10] % 8 == 0 && v[12] != 0 && v[13] != 0 && v[12] != v[13]);
        assert_eq!(v[14], 1);
        assert_eq!(&f[..6], &[2, 1, 2, 0, 1, 1]);
        assert_eq!(f[6], v[10]);
        assert_eq!(f[7], (-1006_i32) as u32);
        assert!(f[8] != 0 && f[8] != v[12]);
        assert_eq!(f[9], f[8]);
        assert_eq!(f[10], 1);
        assert_eq!(f[11], (-1006_i32) as u32);
        assert!(failure_seen && failure_returned && cancel_worker_seen && cancel_returned);
        assert!(machine.cpu.primask && exception == 0);
        assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
        assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
        let rows = trace.rows.lock().unwrap();
        for row in rows.iter() {
            println!(
                "SAMD_REAL_SPI_CANCEL_ROW step={} kind={} value={}",
                row.step, row.kind, row.value
            );
        }
        let split = recovery_start.expect("actual main cancel resume precedes recovery");
        let remaining = cancel_remaining.expect("actual cancellation writeback count captured");
        cancellation_ordering(&rows[..split], split, remaining);
        ordering(&rows[split..]);
        assert_eq!(
            rows.iter().find(|r| r.kind == "failure").unwrap().value,
            f[6]
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.kind == "cancel-worker")
                .unwrap()
                .value,
            f[8]
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.kind == "cancel-return")
                .unwrap()
                .value,
            f[8]
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.kind == "cancel-resume")
                .unwrap()
                .value,
            v[12]
        );
        println!("SAMD_REAL_SPI_CANCEL_RESULT source={source} elf_sha256={image} steps={step} status=0x600d startup=true vectors=true data_copy=true bss_clear=true witnesses={v:?} cancellation={f:?} cancel_rows={split} cancel_remaining={remaining} irq_entries={entries:?} irq_returns={returns:?} actual_swaps={swaps} actual_restores={restores} final_mask=1 final_exception=0 runtime_admission=false");
        terminal = Some(step);
        break;
    }
    assert!(
        terminal.is_some(),
        "actual SPI guest exceeded unchanged bounded budget"
    );
}
