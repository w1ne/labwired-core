// SPDX-License-Identifier: MIT
//! Actual virtual-destruction refusals and settled idle reconstruction only.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, peripherals::spi::SpiDevice, system::cortex_m::configure_cortex_m, Bus,
    Machine, SimulationObserver,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const BUDGET: usize = 4_000_000;
const POST: usize = 32;
const SOURCE: &str = "932d476344107386d56dfd2d595d74d713780563";
#[derive(Clone, Debug, Default)]
struct Context {
    pc: u32,
    lr: u32,
    fiber: u32,
    mask: u32,
    exception: u32,
    regs: [u32; 4],
}
#[derive(Clone, Debug)]
struct Row {
    step: usize,
    kind: &'static str,
    value: u32,
    context: Context,
}
impl Row {
    fn value(&self) -> Value {
        let c = &self.context;
        json!({"step":self.step,"kind":self.kind,"value":self.value,"pc":c.pc,"lr":c.lr,"fiber":c.fiber,"mask":c.mask,"exception":c.exception,"r0_r3":c.regs})
    }
}
#[derive(Debug, Default)]
struct Trace {
    step: Mutex<usize>,
    context: Mutex<Context>,
    rows: Mutex<Vec<Row>>,
    dtor: Mutex<bool>,
    cancel: Mutex<bool>,
}
impl Trace {
    fn record(&self, kind: &'static str, value: u32) {
        self.rows.lock().unwrap().push(Row {
            step: *self.step.lock().unwrap(),
            kind,
            value,
            context: self.context.lock().unwrap().clone(),
        });
    }
}
impl SimulationObserver for Trace {
    fn on_memory_write(&self, address: u64, _old: u8, value: u8) {
        if address == 0x43000028 {
            self.record("accept", value.into());
        }
        if *self.cancel.lock().unwrap() && (0x4100a040..=0x4100a043).contains(&address) {
            self.record(
                [
                    "cancel-mmio0",
                    "cancel-mmio1",
                    "cancel-mmio2",
                    "cancel-mmio3",
                ][(address - 0x4100a040) as usize],
                value.into(),
            );
        }
        if *self.dtor.lock().unwrap() {
            if address == 0x43000014 {
                self.record("dtor-mmio-spi", value.into());
            }
            if address == 0x4100a04c {
                self.record("dtor-mmio-dma", value.into());
            }
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
        self.0.record("wire", byte.into());
        0xff
    }
}
#[derive(Clone, Debug)]
struct Call {
    kind: &'static str,
    returned: &'static str,
    lr: u32,
    fiber: u32,
    done: bool,
}
fn count(rows: &[Row], kind: &str) -> usize {
    rows.iter().filter(|r| r.kind == kind).count()
}
fn at(rows: &[Row], kind: &str) -> usize {
    let hits: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == kind)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(hits.len(), 1, "unique {kind}");
    hits[0]
}
fn predicate(mode: u32, rows: &[Row], g: &[u32; 18], entries: [u32; 3], returns: [u32; 3]) {
    assert!((15..=18).contains(&mode));
    let allowed = [
        "select",
        "accept",
        "wire",
        "release",
        "irq-enter",
        "irq-return",
        "callback",
        "callback-return",
        "async-resume",
        "blocking-call",
        "blocking-return",
        "destroy-call",
        "destroy-return",
        "delete-call",
        "delete-return",
        "d0-call",
        "d0-return",
        "dtor-call",
        "dtor-return",
        "free-call",
        "free-return",
        "remove-call",
        "remove-return",
        "ctor-call",
        "ctor-return",
        "panic",
        "cancel-call",
        "cancel-return",
        "enroll-call",
        "enroll-return",
        "notify-call",
        "notify-return",
        "owner-call",
        "owner-return",
        "canceller-call",
        "canceller-return",
        "cancel-mmio0",
        "cancel-mmio1",
        "cancel-mmio2",
        "cancel-mmio3",
        "dtor-mmio-spi",
        "dtor-mmio-dma",
    ];
    assert!(rows.iter().all(|r| allowed.contains(&r.kind)));
    assert!(rows.windows(2).all(|w| w[0].step <= w[1].step));
    assert_eq!(count(rows, "ctor-call"), if mode == 18 { 2 } else { 1 });
    assert_eq!(count(rows, "ctor-return"), if mode == 18 { 2 } else { 1 });
    let expected_select = if mode == 18 { 4 } else { 1 };
    assert_eq!(count(rows, "select"), expected_select);
    assert_eq!(
        count(rows, "release"),
        if mode == 15 { 0 } else { expected_select }
    );
    assert_eq!(
        count(rows, "irq-enter"),
        entries.iter().sum::<u32>() as usize
    );
    assert_eq!(
        count(rows, "irq-return"),
        returns.iter().sum::<u32>() as usize
    );
    for (kind, census) in [("irq-enter", entries), ("irq-return", returns)] {
        for (i, value) in [15, 47, 79].into_iter().enumerate() {
            assert_eq!(
                rows.iter()
                    .filter(|r| r.kind == kind && r.value == value)
                    .count(),
                census[i] as usize
            );
        }
        assert!(rows
            .iter()
            .filter(|r| r.kind == kind)
            .all(|r| [15, 47, 79].contains(&r.value)));
    }
    assert_eq!(
        count(rows, "callback"),
        if mode == 18 {
            2
        } else {
            usize::from(mode == 16)
        }
    );
    assert_eq!(
        count(rows, "callback-return"),
        if mode == 18 { 2 } else { 0 }
    );
    for kind in [
        "owner-call",
        "canceller-call",
        "cancel-call",
        "cancel-return",
        "enroll-call",
        "enroll-return",
        "notify-call",
        "notify-return",
        "cancel-mmio0",
        "cancel-mmio1",
        "cancel-mmio2",
        "cancel-mmio3",
    ] {
        assert_eq!(count(rows, kind), usize::from(mode == 17), "closed {kind}");
    }
    for kind in ["async-resume", "blocking-call", "blocking-return"] {
        assert_eq!(
            count(rows, kind),
            if mode == 18 { 2 } else { 0 },
            "closed {kind}"
        );
    }
    assert_eq!(g[14], 0);
    assert_eq!(g[13], 0);
    for (a, b) in [
        ("destroy-call", "delete-call"),
        ("delete-call", "d0-call"),
        ("d0-call", "dtor-call"),
    ] {
        assert!(at(rows, a) < at(rows, b));
    }
    if mode < 18 {
        assert_eq!(g[17], 0xc42);
        assert_eq!(g[6], 0);
        assert_eq!(g[2], 0);
        assert_eq!(g[0], 3);
        for kind in [
            "destroy-return",
            "delete-return",
            "d0-return",
            "dtor-return",
            "free-call",
            "free-return",
            "remove-call",
            "remove-return",
            "dtor-mmio-spi",
            "dtor-mmio-dma",
            "owner-return",
            "canceller-return",
        ] {
            assert_eq!(count(rows, kind), 0, "no {kind} on refusal");
        }
        let p = &rows[at(rows, "panic")];
        assert_eq!(p.value, 0xc42);
        assert_eq!(p.context.mask, 1);
        assert!(at(rows, "dtor-call") < at(rows, "panic"));
        if mode == 16 {
            assert_eq!(entries, [0, 1, 1]);
            assert_eq!(returns, [0, 1, 0]);
            assert_eq!(p.context.exception, 79);
            assert_eq!(g[5], 79);
            assert_eq!(g[4], 0);
            assert_eq!(count(rows, "callback"), 1);
            assert_eq!(count(rows, "callback-return"), 0);
            assert!(at(rows, "callback") < at(rows, "destroy-call"));
        } else {
            assert_eq!(entries, [0, 0, 0]);
            assert_eq!(returns, [0, 0, 0]);
            assert_eq!(p.context.exception, 0);
            assert_eq!(g[4], 1);
            assert_eq!(g[5], 0);
            assert_eq!(count(rows, "callback"), 0);
        }
        if mode == 17 {
            let epoch = [
                "owner-call",
                "enroll-call",
                "enroll-return",
                "select",
                "canceller-call",
                "cancel-call",
                "cancel-mmio0",
                "cancel-mmio1",
                "cancel-mmio2",
                "cancel-mmio3",
                "release",
                "notify-call",
                "notify-return",
                "cancel-return",
                "destroy-call",
            ];
            for pair in epoch.windows(2) {
                assert!(at(rows, pair[0]) < at(rows, pair[1]));
            }
            for (role, value) in [
                ("cancel-mmio0", 0),
                ("cancel-mmio1", 13),
                ("cancel-mmio2", 32),
                ("cancel-mmio3", 0),
            ] {
                assert_eq!(rows[at(rows, role)].value, value);
            }
            assert_eq!(g[12], 0);
            assert_ne!(g[10], g[11]);
            assert_eq!(g[3], g[11]);
            assert!(g[16] > 0 && g[16] < 32);
        }
    } else {
        assert_eq!(g[17], 0);
        assert_eq!(g[0], 6);
        assert_eq!(g[6], 1);
        assert_eq!(g[4], 1);
        assert_eq!(g[5], 0);
        assert_eq!(g[7], 1);
        assert_eq!(g[8], 0);
        assert_eq!(g[9], g[3]);
        assert_ne!(g[2], 0);
        assert_eq!(entries, [0, 4, 4]);
        assert_eq!(returns, entries);
        for kind in [
            "panic",
            "owner-call",
            "cancel-call",
            "canceller-call",
            "enroll-call",
            "notify-call",
        ] {
            assert_eq!(count(rows, kind), 0);
        }
        for (a, b) in [
            ("dtor-call", "remove-call"),
            ("remove-call", "remove-return"),
            ("remove-return", "dtor-return"),
            ("dtor-return", "free-call"),
            ("free-call", "free-return"),
            ("free-return", "d0-return"),
            ("d0-return", "delete-return"),
            ("delete-return", "destroy-return"),
        ] {
            assert!(at(rows, a) < at(rows, b));
        }
        assert_eq!(count(rows, "ctor-call"), 2);
        assert_eq!(count(rows, "ctor-return"), 2);
        assert_eq!(count(rows, "callback"), 2);
        assert_eq!(count(rows, "callback-return"), 2);
        assert_eq!(count(rows, "select"), 4);
        assert_eq!(count(rows, "release"), 4);
        assert_eq!(count(rows, "accept"), 16);
        assert_eq!(count(rows, "wire"), 16);
        assert_eq!(count(rows, "dtor-mmio-spi"), 1);
        assert_eq!(rows[at(rows, "dtor-mmio-spi")].value, 255);
        assert_eq!(count(rows, "dtor-mmio-dma"), 1);
        assert_eq!(rows[at(rows, "dtor-mmio-dma")].value, 7);
        let select: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == "select")
            .map(|(i, _)| i)
            .collect();
        let release: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == "release")
            .map(|(i, _)| i)
            .collect();
        let blocking: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == "blocking-return")
            .map(|(i, _)| i)
            .collect();
        let ctor: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == "ctor-call")
            .map(|(i, _)| i)
            .collect();
        assert!(
            release[1] < blocking[0]
                && blocking[0] < at(rows, "destroy-call")
                && at(rows, "destroy-return") < ctor[1]
                && ctor[1] < select[2]
                && release[3] < blocking[1]
        );
    }
    let accepted: Vec<_> = rows.iter().filter(|r| r.kind == "accept").collect();
    let wire: Vec<_> = rows.iter().filter(|r| r.kind == "wire").collect();
    assert!(wire.len() <= accepted.len());
    let payload = if mode == 15 || mode == 17 {
        [0x91, 0x82, 0x73, 0x64]
            .into_iter()
            .chain(std::iter::repeat(0))
            .take(32)
            .collect::<Vec<_>>()
    } else {
        [0x12, 0x34, 0x56, 0x78].repeat(if mode == 18 { 4 } else { 1 })
    };
    assert!(accepted.len() <= payload.len());
    for (i, r) in accepted.iter().enumerate() {
        assert_eq!(r.value, payload[i]);
    }
    for (i, r) in wire.iter().enumerate() {
        assert_eq!(r.value, payload[i]);
        assert!(accepted[i].step <= r.step);
    }
    if mode == 16 || mode == 18 {
        let phases = if mode == 18 { 4 } else { 1 };
        assert_eq!(accepted.len(), phases * 4, "complete accepted bytes");
        assert_eq!(wire.len(), phases * 4, "complete wire bytes");
        let indices = |kind: &str, value: Option<u32>| {
            rows.iter()
                .enumerate()
                .filter(|(_, r)| r.kind == kind && value.is_none_or(|v| r.value == v))
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        let select = indices("select", None);
        let release = indices("release", None);
        let accept = indices("accept", None);
        let wire = indices("wire", None);
        let dma = indices("irq-enter", Some(47));
        let dma_return = indices("irq-return", Some(47));
        let txc = indices("irq-enter", Some(79));
        let txc_return = indices("irq-return", Some(79));
        let callback = indices("callback", None);
        let callback_return = indices("callback-return", None);
        let resumed = indices("async-resume", None);
        let blocking_call = indices("blocking-call", None);
        let blocking_return = indices("blocking-return", None);
        for phase in 0..phases {
            assert!(
                select[phase] < accept[phase * 4]
                    && accept[phase * 4 + 3] < dma[phase]
                    && dma[phase] < wire[phase * 4 + 3]
                    && wire[phase * 4 + 3] < txc[phase]
                    && txc[phase] < release[phase],
                "DMA/wire/TXC/release phase{phase}"
            );
            assert!(dma[phase] < dma_return[phase] && dma_return[phase] < txc[phase]);
            for byte in phase * 4..phase * 4 + 4 {
                assert!(
                    select[phase] < accept[byte]
                        && accept[byte] < wire[byte]
                        && wire[byte] < release[phase]
                );
            }
            if mode == 18 {
                assert!(release[phase] < txc_return[phase]);
                if phase.is_multiple_of(2) {
                    let i = phase / 2;
                    assert!(
                        release[phase] < callback[i]
                            && callback[i] < callback_return[i]
                            && callback_return[i] < txc_return[phase]
                            && txc_return[phase] < resumed[i]
                            && resumed[i] < blocking_call[i]
                            && blocking_call[i] < select[phase + 1]
                    );
                } else {
                    assert!(txc_return[phase] < blocking_return[phase / 2]);
                }
            } else {
                assert!(release[0] < callback[0] && callback[0] < at(rows, "destroy-call"));
            }
        }
    }
    if mode == 17 {
        assert_eq!(accepted.len(), 32 - g[16] as usize);
        let release = at(rows, "release");
        assert!(rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == "accept" || r.kind == "wire")
            .all(|(i, _)| i < release));
    }
    if mode == 15 {
        assert!(g[16] > 0 && g[16] < 32);
        assert!(accepted.len() >= 32 - g[16] as usize);
    }
}

#[test]
fn manufactured_lifetime_panic_epoch_refuses() {
    let c = Context {
        mask: 1,
        ..Default::default()
    };
    let roles = [
        "ctor-call",
        "ctor-return",
        "select",
        "accept",
        "destroy-call",
        "delete-call",
        "d0-call",
        "dtor-call",
        "panic",
    ];
    let rows: Vec<_> = roles
        .iter()
        .enumerate()
        .map(|(i, k)| Row {
            step: i + 1,
            kind: k,
            value: if *k == "panic" {
                0xc42
            } else if *k == "accept" {
                0x91
            } else {
                0
            },
            context: c.clone(),
        })
        .collect();
    let mut g = [0; 18];
    g[0] = 3;
    g[4] = 1;
    g[16] = 31;
    g[17] = 0xc42;
    predicate(15, &rows, &g, [0; 3], [0; 3]);
    for i in 0..rows.len() {
        let mut bad = rows.clone();
        bad.remove(i);
        assert!(std::panic::catch_unwind(|| predicate(15, &bad, &g, [0; 3], [0; 3])).is_err());
    }
    for (kind, value) in [("panic", 0xe33), ("free-call", 0), ("unknown", 0)] {
        let mut bad = rows.clone();
        if kind == "panic" {
            bad[8].value = value;
        } else {
            bad.push(Row {
                step: 9,
                kind,
                value,
                context: c.clone(),
            });
        }
        assert!(std::panic::catch_unwind(|| predicate(15, &bad, &g, [0; 3], [0; 3])).is_err());
    }
}

#[test]
fn manufactured_all_lifetime_modes_refuse_missing_or_extra_effects() {
    for mode in 15..=18 {
        let mut raw = vec![("ctor-call", 0), ("ctor-return", 0)];
        let mut g = [0; 18];
        g[0] = 3;
        g[4] = 1;
        g[16] = 30;
        g[17] = 0xc42;
        let mut entries = [0; 3];
        let mut returns = [0; 3];
        if mode == 17 {
            raw.extend([("owner-call", 0), ("enroll-call", 7), ("enroll-return", 0)]);
            g[10] = 1;
            g[11] = 2;
            g[3] = 2;
        }
        if mode == 15 || mode == 17 {
            raw.extend([("select", 0), ("accept", 0x91), ("accept", 0x82)]);
        }
        if mode == 16 {
            raw.push(("select", 0));
            for (i, value) in [0x12, 0x34, 0x56, 0x78].into_iter().enumerate() {
                raw.push(("accept", value));
                if i == 3 {
                    raw.extend([("irq-enter", 47), ("irq-return", 47)]);
                }
                raw.push(("wire", value));
            }
            raw.extend([("irq-enter", 79), ("release", 0), ("callback", 0)]);
            g[4] = 0;
            g[5] = 79;
            entries = [0, 1, 1];
            returns = [0, 1, 0];
        }
        if mode == 17 {
            raw.extend([
                ("canceller-call", 0),
                ("cancel-call", 0),
                ("cancel-mmio0", 0),
                ("cancel-mmio1", 13),
                ("cancel-mmio2", 32),
                ("cancel-mmio3", 0),
                ("release", 0),
                ("notify-call", 7),
                ("notify-return", 0),
                ("cancel-return", 0),
            ]);
        }
        let cycle = |raw: &mut Vec<(&'static str, u32)>| {
            for phase in 0..2 {
                if phase == 1 {
                    raw.push(("blocking-call", 0));
                }
                raw.push(("select", 0));
                for (i, value) in [0x12, 0x34, 0x56, 0x78].into_iter().enumerate() {
                    raw.push(("accept", value));
                    if i == 3 {
                        raw.extend([("irq-enter", 47), ("irq-return", 47)]);
                    }
                    raw.push(("wire", value));
                }
                raw.extend([("irq-enter", 79), ("release", 0)]);
                if phase == 0 {
                    raw.extend([("callback", 0), ("callback-return", 0)]);
                }
                raw.push(("irq-return", 79));
                raw.push((
                    if phase == 0 {
                        "async-resume"
                    } else {
                        "blocking-return"
                    },
                    0,
                ));
            }
        };
        if mode == 18 {
            cycle(&mut raw);
            g[0] = 6;
            g[6] = 1;
            g[7] = 1;
            g[17] = 0;
            g[2] = 1;
            entries = [0, 4, 4];
            returns = entries;
        }
        raw.extend([
            ("destroy-call", 0),
            ("delete-call", 0),
            ("d0-call", 0),
            ("dtor-call", 0),
        ]);
        if mode == 18 {
            raw.extend([
                ("dtor-mmio-spi", 255),
                ("dtor-mmio-dma", 7),
                ("remove-call", 0),
                ("remove-return", 0),
                ("dtor-return", 0),
                ("free-call", 0),
                ("free-return", 0),
                ("d0-return", 0),
                ("delete-return", 0),
                ("destroy-return", 0),
                ("ctor-call", 0),
                ("ctor-return", 0),
            ]);
            cycle(&mut raw);
        } else {
            raw.push(("panic", 0xc42));
        }
        let context = Context {
            mask: 1,
            exception: if mode == 16 { 79 } else { 0 },
            ..Default::default()
        };
        let rows: Vec<_> = raw
            .into_iter()
            .enumerate()
            .map(|(i, (kind, value))| Row {
                step: i + 1,
                kind,
                value,
                context: context.clone(),
            })
            .collect();
        predicate(mode, &rows, &g, entries, returns);
        if mode == 16 || mode == 18 {
            let last_wire = rows
                .iter()
                .position(|r| r.kind == "wire" && r.value == 0x78)
                .unwrap();
            let dma = rows
                .iter()
                .position(|r| r.kind == "irq-enter" && r.value == 47)
                .unwrap();
            let txc = rows
                .iter()
                .position(|r| r.kind == "irq-enter" && r.value == 79)
                .unwrap();
            let release = rows.iter().position(|r| r.kind == "release").unwrap();
            for (a, b) in [(dma, last_wire), (txc, last_wire), (txc, release)] {
                let mut bad = rows.clone();
                bad.swap(a, b);
                for (i, row) in bad.iter_mut().enumerate() {
                    row.step = i + 1;
                }
                assert!(
                    std::panic::catch_unwind(|| predicate(mode, &bad, &g, entries, returns))
                        .is_err(),
                    "wrong physical epoch mode{mode}"
                );
            }
        }
        for i in 0..rows.len() {
            if rows[i].kind == "wire" && (mode == 15 || mode == 17) {
                continue;
            }
            let mut bad = rows.clone();
            bad.remove(i);
            assert!(
                std::panic::catch_unwind(|| predicate(mode, &bad, &g, entries, returns)).is_err(),
                "mode{mode} missing{}",
                rows[i].kind
            );
        }
        for kind in ["unknown", "accept", "callback", "free-return", "panic"] {
            let mut bad = rows.clone();
            bad.push(Row {
                step: rows.len() + 1,
                kind,
                value: 0,
                context: context.clone(),
            });
            assert!(
                std::panic::catch_unwind(|| predicate(mode, &bad, &g, entries, returns)).is_err()
            );
        }
    }
}

#[test]
#[ignore = "actual source-bound SPI destruction callers require dedicated hosted builds and both scheduler profiles"]
fn actual_codal_spi_lifetime_and_idle_reconstruction() {
    let mode: u32 = std::env::var("LABWIRED_SPI_MODE").unwrap().parse().unwrap();
    assert!((15..=18).contains(&mode));
    assert_eq!(std::env::var("LABWIRED_SPI_SOURCE").unwrap(), SOURCE);
    let folder = PathBuf::from(std::env::var("LABWIRED_SPI_FIXTURE_DIR").unwrap());
    let provenance: Value =
        serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(provenance["source"], SOURCE);
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
    let symbol = |name: &str| {
        let matches: Vec<_> = elf
            .syms
            .iter()
            .filter(|s| s.st_shndx != 0 && elf.strtab.get_at(s.st_name) == Some(name))
            .collect();
        assert_eq!(matches.len(), 1, "unique {name}");
        matches[0].st_value
    };
    let aliases = |prefixes: &[&str]| {
        let mut targets: Vec<_> = elf
            .syms
            .iter()
            .filter(|s| s.st_shndx != 0)
            .filter_map(|s| {
                let n = elf.strtab.get_at(s.st_name)?;
                prefixes
                    .iter()
                    .any(|p| n.starts_with(p))
                    .then_some(s.st_value as u32 & !1)
            })
            .collect();
        targets.sort_unstable();
        targets.dedup();
        assert_eq!(targets.len(), 1, "one alias target {prefixes:?}");
        targets[0]
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
    let lifetime_names = [
        "stage",
        "object",
        "fresh",
        "actor",
        "mask",
        "ipsr",
        "returned",
        "return_mask",
        "return_ipsr",
        "return_actor",
        "owner_identity",
        "canceller_identity",
        "cancel_result",
        "owner_returned",
        "failures",
        "writeback",
        "remaining",
        "expected_panic",
    ]
    .map(|n| format!("runtime_spi_lifetime_{n}"));
    let lifetime_addresses = lifetime_names.each_ref().map(|n| symbol(n));
    let fiber_address = symbol("_ZN5codal12currentFiberE");
    let d0 = aliases(&["_ZN19brickwright_runtime12Sercom4TxSPID0E"]);
    let dtor = aliases(&[
        "_ZN19brickwright_runtime12Sercom4TxSPID1E",
        "_ZN19brickwright_runtime12Sercom4TxSPID2E",
    ]);
    assert_ne!(
        d0, dtor,
        "distinct deleting and complete destructor targets"
    );
    let ctor = aliases(&[
        "_ZN19brickwright_runtime12Sercom4TxSPIC1E",
        "_ZN19brickwright_runtime12Sercom4TxSPIC2E",
    ]);
    let remove = aliases(&["_ZN5codal10MessageBus6removeE"]);
    let event = aliases(&["_ZN5codal5EventC1EttN", "_ZN5codal5EventC2EttN"]);
    let enroll = symbol("_ZN5codal19fiber_wake_on_eventEtt") as u32 & !1;
    let fixed = [
        (
            "destroy-call",
            "destroy-return",
            symbol("runtime_spi_lifetime_destroy") as u32 & !1,
        ),
        (
            "delete-call",
            "delete-return",
            symbol("runtime_spi_lifetime_delete") as u32 & !1,
        ),
        ("d0-call", "d0-return", d0),
        ("dtor-call", "dtor-return", dtor),
        ("ctor-call", "ctor-return", ctor),
        (
            "cancel-call",
            "cancel-return",
            symbol("runtime_spi_lifetime_cancel") as u32 & !1,
        ),
        (
            "owner-call",
            "owner-return",
            symbol("runtime_spi_lifetime_owner") as u32 & !1,
        ),
        (
            "canceller-call",
            "canceller-return",
            symbol("runtime_spi_lifetime_canceller") as u32 & !1,
        ),
        (
            "blocking-call",
            "blocking-return",
            symbol("runtime_spi_lifetime_blocking_worker") as u32 & !1,
        ),
    ];
    let bss = symbol("_bss_start")..symbol("_bss_end");
    let data = symbol("_data_start")..symbol("_data_end");
    let mut distinct = [
        addresses.to_vec(),
        lifetime_addresses.to_vec(),
        vec![fiber_address],
    ]
    .concat();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 34);
    assert!(distinct
        .iter()
        .all(|a| bss.contains(a) && a.is_multiple_of(4)));
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
        let w = machine.bus.read_u32(vector * 4).unwrap();
        let target = match vector {
            1 => "reset_entry",
            15 => "runtime_systick",
            47 => "runtime_dma_irq",
            79 => "runtime_spi_irq",
            _ => "runtime_fault",
        };
        assert_eq!(w & 1, 1);
        assert_eq!(u64::from(w & !1), symbol(target) & !1);
    }
    let initial: Vec<_> = (0..data.end - data.start)
        .step_by(4)
        .map(|off| machine.bus.read_u32(symbol("_data_load") + off).unwrap())
        .collect();
    for a in bss.clone().step_by(4) {
        machine.bus.write_u32(a, 0xa5a5a5a5).unwrap();
    }
    for a in data.clone().step_by(4) {
        machine.bus.write_u32(a, 0x5a5a5a5a).unwrap();
    }
    let mut startup = false;
    let mut calls: Vec<Call> = vec![];
    let mut entries = [0u32; 3];
    let mut returns = [0u32; 3];
    let mut previous = 0;
    let mut swaps = 0;
    let mut restores = 0;
    let mut terminal_step = None;
    let mut terminal_context = None;
    let mut panic_reason = 0u32;
    let mut notified = 0u32;
    let mut enrollment = 0u32;
    let mut last_stage = 0;
    let mut async_stage = 0;
    let mut teardown_reads = Value::Null;
    let mut teardown_before = Value::Null;
    for step in 1..=BUDGET {
        *trace.step.lock().unwrap() = step;
        let pc = machine.cpu.pc & !1;
        let fiber = if startup {
            machine.bus.read_u32(fiber_address).unwrap()
        } else {
            0
        };
        let c = Context {
            pc,
            lr: machine.cpu.lr,
            fiber,
            mask: u32::from(machine.cpu.primask),
            exception: machine.cpu.active_exception,
            regs: [
                machine.cpu.r0,
                machine.cpu.r1,
                machine.cpu.r2,
                machine.cpu.r3,
            ],
        };
        *trace.context.lock().unwrap() = c.clone();
        let g = lifetime_addresses.map(|a| machine.bus.read_u32(a).unwrap());
        // Actual call-return matching includes live fiber identity. D1/D2 and
        // C1/C2 aliases resolve to one real target, never invented entry rows.
        for call in calls.iter_mut().rev() {
            if !call.done && pc == call.lr && fiber == call.fiber {
                call.done = true;
                trace.record(call.returned, machine.cpu.r0);
                if call.kind == "cancel-call" {
                    *trace.cancel.lock().unwrap() = false;
                    assert_eq!(machine.cpu.r0, 0);
                }
                if call.kind == "dtor-call" {
                    *trace.dtor.lock().unwrap() = false;
                    teardown_reads = json!({"step":step,"nvic_enabled0":machine.bus.read_u32(0xe000e100).unwrap(),"nvic_enabled1":machine.bus.read_u32(0xe000e104).unwrap(),"spi_inten":machine.bus.read_u8(0x43000016).unwrap(),"dma_inten":machine.bus.read_u8(0x4100a04d).unwrap()});
                }
            }
        }
        if startup {
            let mut selected = fixed.iter().find(|(_, _, target)| *target == pc).copied();
            let in_dtor = calls.iter().any(|c| c.kind == "dtor-call" && !c.done);
            let in_d0 = calls.iter().any(|c| c.kind == "d0-call" && !c.done);
            if in_dtor && pc == remove {
                selected = Some(("remove-call", "remove-return", remove));
            }
            if in_d0 && pc == (symbol("free") as u32 & !1) && machine.cpu.r0 == g[1] {
                selected = Some(("free-call", "free-return", pc));
            }
            if mode == 17 && pc == enroll && fiber == g[10] && g[10] != 0 {
                assert_eq!(machine.cpu.r0, 1023);
                assert_eq!(c.mask, 1);
                enrollment = machine.cpu.r1;
                selected = Some(("enroll-call", "enroll-return", pc));
            }
            if mode == 17 && pc == event && calls.iter().any(|c| c.kind == "cancel-call" && !c.done)
            {
                assert_eq!(
                    (machine.cpu.r1, machine.cpu.r2, machine.cpu.r3),
                    (1022, enrollment, 1)
                );
                assert!(enrollment > 0 && enrollment <= 65535);
                notified = machine.cpu.r2;
                selected = Some(("notify-call", "notify-return", pc));
            }
            if let Some((kind, returned, _target)) = selected {
                assert!(
                    !calls.iter().any(|c| c.kind == kind && !c.done),
                    "unexpected recursive {kind}"
                );
                calls.push(Call {
                    kind,
                    returned,
                    lr: machine.cpu.lr & !1,
                    fiber,
                    done: false,
                });
                trace.record(kind, machine.cpu.r0);
                if kind == "cancel-call" {
                    *trace.cancel.lock().unwrap() = true;
                }
                if kind == "dtor-call" {
                    *trace.dtor.lock().unwrap() = true;
                    teardown_before = json!({"step":step,"nvic_enabled0":machine.bus.read_u32(0xe000e100).unwrap(),"nvic_enabled1":machine.bus.read_u32(0xe000e104).unwrap(),"spi_inten":machine.bus.read_u8(0x43000016).unwrap(),"dma_inten":machine.bus.read_u8(0x4100a04d).unwrap()});
                }
            }
            if pc == (symbol("runtime_spi_done") as u32 & !1) {
                calls.push(Call {
                    kind: "callback",
                    returned: "callback-return",
                    lr: machine.cpu.lr & !1,
                    fiber,
                    done: false,
                });
                trace.record("callback", machine.cpu.r0);
            }
            if pc == (symbol("target_panic") as u32 & !1) {
                assert!(terminal_step.is_none(), "duplicate panic entry");
                assert!(mode < 18);
                assert_eq!(
                    machine.cpu.r0, 0xc42,
                    "only exact deliberate destructor refusal"
                );
                assert_eq!(c.mask, 1);
                assert_eq!(c.exception, if mode == 16 { 79 } else { 0 });
                panic_reason = machine.cpu.r0;
                terminal_step = Some(step);
                terminal_context = Some(c.clone());
                trace.record("panic", machine.cpu.r0);
            }
        }
        if pc == (symbol("swap_context") as u32 & !1) {
            swaps += 1;
        }
        if pc == (symbol("restore_register_context") as u32 & !1) {
            restores += 1;
        }
        if let Err(error) = machine.step() {
            println!(
                "SAMD_REAL_SPI_LIFETIME_STEP_ERROR mode={mode} step={step} error={error:?} rows={}",
                json!(trace
                    .rows
                    .lock()
                    .unwrap()
                    .iter()
                    .map(Row::value)
                    .collect::<Vec<_>>())
            );
            panic!("actual lifetime step fault is not refusal");
        }
        if !startup && machine.cpu.pc & !1 == symbol("runtime_probe") as u32 & !1 {
            for a in bss.clone().step_by(4) {
                assert_eq!(machine.bus.read_u32(a).unwrap(), 0);
            }
            for (a, w) in data.clone().step_by(4).zip(&initial) {
                assert_eq!(machine.bus.read_u32(a).unwrap(), *w);
            }
            assert!(machine.cpu.primask && machine.cpu.active_exception == 0);
            startup = true;
        }
        let exception = machine.cpu.active_exception;
        assert!([0, 15, 47, 79].contains(&exception));
        if previous != exception {
            *trace.context.lock().unwrap() = Context {
                pc: machine.cpu.pc & !1,
                lr: machine.cpu.lr,
                fiber: if startup {
                    machine.bus.read_u32(fiber_address).unwrap()
                } else {
                    0
                },
                mask: u32::from(machine.cpu.primask),
                exception,
                regs: [
                    machine.cpu.r0,
                    machine.cpu.r1,
                    machine.cpu.r2,
                    machine.cpu.r3,
                ],
            };
            if previous != 0 {
                returns[[15, 47, 79].iter().position(|x| *x == previous).unwrap()] += 1;
                trace.record("irq-return", previous);
            }
            if exception != 0 {
                entries[[15, 47, 79].iter().position(|x| *x == exception).unwrap()] += 1;
                trace.record("irq-enter", exception);
            }
        }
        previous = exception;
        if !startup {
            continue;
        }
        let v = addresses.map(|a| machine.bus.read_u32(a).unwrap());
        let g = lifetime_addresses.map(|a| machine.bus.read_u32(a).unwrap());
        if mode == 18 && [2, 4].contains(&v[6]) && v[6] != async_stage {
            async_stage = v[6];
            assert_eq!(machine.bus.read_u32(fiber_address).unwrap(), v[12]);
            assert!(calls
                .iter()
                .filter(|c| c.kind == "callback")
                .all(|c| c.done));
            *trace.context.lock().unwrap() = Context {
                pc: machine.cpu.pc & !1,
                lr: machine.cpu.lr,
                fiber: v[12],
                mask: u32::from(machine.cpu.primask),
                exception,
                regs: [
                    machine.cpu.r0,
                    machine.cpu.r1,
                    machine.cpu.r2,
                    machine.cpu.r3,
                ],
            };
            trace.record("async-resume", v[12]);
        }
        if g[0] != last_stage {
            last_stage = g[0];
            println!(
                "SAMD_REAL_SPI_LIFETIME_DIAGNOSTIC {}",
                json!({"mode":mode,"step":step,"source":SOURCE,"image":image,"baseline":v,"lifetime":g,"entries":entries,"returns":returns,"pc":machine.cpu.pc,"mask":machine.cpu.primask,"exception":exception})
            );
        }
        if mode == 18 && v[0] == 0x600d && terminal_step.is_none() {
            assert_eq!(g[0], 6);
            terminal_step = Some(step);
            terminal_context = Some(Context {
                pc: machine.cpu.pc & !1,
                lr: machine.cpu.lr,
                fiber: machine.bus.read_u32(fiber_address).unwrap(),
                mask: u32::from(machine.cpu.primask),
                exception,
                regs: [
                    machine.cpu.r0,
                    machine.cpu.r1,
                    machine.cpu.r2,
                    machine.cpu.r3,
                ],
            });
        }
        assert!(
            [0, 0x100, 0x600d, 0xe0000c42].contains(&v[0]),
            "unexpected actual status {:#x}",
            v[0]
        );
        if let Some(t) = terminal_step {
            if step >= t + POST {
                assert_eq!(v[0], if mode == 18 { 0x600d } else { 0xe0000c42 });
                assert_eq!(v[3], swaps);
                assert_eq!(v[4], restores);
                assert_eq!(v[5], entries[0]);
                let cfsr = machine.bus.read_u32(0xe000ed28).unwrap();
                let hfsr = machine.bus.read_u32(0xe000ed2c).unwrap();
                assert_eq!(cfsr, 0);
                assert_eq!(hfsr, 0);
                assert_eq!(
                    &v[..3],
                    &[
                        if mode == 18 { 0x600d } else { 0xe0000c42 },
                        if mode == 18 { 3 } else { 1 },
                        1
                    ]
                );
                if mode == 18 {
                    assert_eq!(&v[6..10], &[4, 2, 79, 1]);
                    assert_eq!(v[10], g[2]);
                    assert_eq!(v[11], (-1006_i32) as u32);
                    assert_eq!(v[14], 2);
                } else {
                    assert_eq!(v[6], 0);
                    assert_eq!(v[7], u32::from(mode == 16));
                    assert_eq!(v[14], 0);
                    if mode == 16 {
                        assert_eq!(&v[8..11], &[79, 1, g[1]]);
                    } else {
                        assert_eq!(&v[8..12], &[0, 0, 0, 0]);
                    }
                }
                let rows = trace.rows.lock().unwrap();
                for row in rows.iter() {
                    println!("SAMD_REAL_SPI_LIFETIME_ROW {}", row.value());
                }
                predicate(mode, &rows, &g, entries, returns);
                if mode < 18 {
                    assert!(calls.iter().any(|c| c.kind == "dtor-call" && !c.done));
                    assert_eq!(g[3], terminal_context.as_ref().unwrap().fiber);
                } else {
                    assert_eq!(teardown_reads["spi_inten"], 0);
                    assert_eq!(teardown_reads["dma_inten"], 0);
                    for field in ["nvic_enabled0", "nvic_enabled1"] {
                        assert_ne!(teardown_before[field].as_u64().unwrap() & (1 << 31), 0);
                        assert_eq!(teardown_reads[field].as_u64().unwrap() & (1 << 31), 0);
                    }
                }
                let c = terminal_context.unwrap();
                let symbols = json!({"destroy":symbol("runtime_spi_lifetime_destroy") as u32&!1,"delete":symbol("runtime_spi_lifetime_delete") as u32&!1,"d0":d0,"dtor":dtor,"free":symbol("free") as u32&!1,"remove":remove,"ctor":ctor,"panic":symbol("target_panic") as u32&!1,"cancel":symbol("runtime_spi_lifetime_cancel") as u32&!1,"enroll":enroll,"notify":event,"owner":symbol("runtime_spi_lifetime_owner") as u32&!1,"canceller":symbol("runtime_spi_lifetime_canceller") as u32&!1,"callback":symbol("runtime_spi_done") as u32&!1,"blocking":symbol("runtime_spi_lifetime_blocking_worker") as u32&!1});
                println!(
                    "SAMD_REAL_SPI_LIFETIME_RESULT {}",
                    json!({"schema":1,"mode":mode,"profile":if labwired_core::cycle_clock::event_scheduler_enabled(){"event-scheduler"}else{"default"},"source":SOURCE,"image":image,"provenance":provenance,"configs":{"chip":"configs/chips/atsamd51-pybadge.yaml","system":"configs/systems/pybadge.yaml"},"steps":step,"budget":BUDGET,"post_terminal_steps":POST,"terminal":{"step":terminal_step,"kind":if mode==18 {"idle-success"}else{"refused"},"panic_reason":panic_reason,"pc":c.pc,"lr":c.lr,"fiber":c.fiber,"mask":c.mask,"exception":c.exception,"r0_r3":c.regs},"startup":true,"vectors":true,"data_copy":true,"bss_clear":true,"witnesses":{"baseline":v,"lifetime":g},"irq_entries":entries,"irq_returns":returns,"swaps":swaps,"restores":restores,"cfsr":cfsr,"hfsr":hfsr,"notify_event":notified,"teardown_before":teardown_before,"teardown_reads":teardown_reads,"symbols":symbols,"rows":rows.iter().map(Row::value).collect::<Vec<_>>() })
                );
                return;
            }
        }
    }
    panic!("actual lifetime case exceeded unchanged bound; never a refusal");
}
