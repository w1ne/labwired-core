// SPDX-License-Identifier: MIT
//! Prospective actual CODAL services, not whole native Arcade admission.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine, SimulationObserver,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

const CORE: &str = "2ed19d64ec3359143409a4b0a9758b4aeef3ee7d";
const NAMES: &[&str] = &[
    "runtime_status",
    "runtime_checks",
    "runtime_heap_reused",
    "runtime_switches",
    "runtime_restores",
    "runtime_clock_irqs",
    "runtime_listener_calls",
    "runtime_listener_exception",
    "runtime_fiber_started",
    "runtime_fiber_woken",
    "runtime_main_woken",
    "runtime_main_identity",
    "runtime_worker_identity",
];
#[derive(Debug)]
struct Calls {
    swap: u32,
    restore: u32,
    swaps: AtomicU32,
    restores: AtomicU32,
}
impl SimulationObserver for Calls {
    fn on_step_start(&self, pc: u32, _opcode: u32) {
        if pc == self.swap {
            self.swaps.fetch_add(1, Ordering::Relaxed);
        }
        if pc == self.restore {
            self.restores.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn positive(v: &[u32], entries: u32, returns: u32, swaps: u32, restores: u32) {
    assert_eq!(v.len(), NAMES.len());
    assert_eq!(v[0..3], [0x600d, 3, 1]);
    assert!(swaps >= 3 && v[3] == swaps, "actual assembly switches");
    assert_eq!(v[4], restores);
    assert!(entries > 0 && entries == returns && v[5] == entries);
    assert_eq!(v[6..11], [1, 15, 1, 1, 1]);
    assert!(v[11] != 0 && v[12] != 0 && v[11] != v[12]);
}
#[test]
fn manufactured_missing_services_witness_refuses() {
    let good = [
        0x600d, 3, 1, 3, 0, 5, 1, 15, 1, 1, 1, 0x20001000, 0x20001100,
    ];
    positive(&good, 5, 5, 3, 0);
    for index in 0..good.len() {
        let mut bad = good;
        bad[index] = if index == 4 { 1 } else { 0 };
        assert!(std::panic::catch_unwind(|| positive(&bad, 5, 5, 3, 0)).is_err());
    }
    for (entries, returns, swaps, restores) in
        [(0, 5, 3, 0), (5, 4, 3, 0), (5, 5, 2, 0), (5, 5, 3, 1)]
    {
        assert!(
            std::panic::catch_unwind(|| positive(&good, entries, returns, swaps, restores))
                .is_err()
        );
    }
}

// An exact old-allocator bug witness, never a positive runtime admission.
fn heap_alignment_bug(
    v: &[u32],
    a: [u32; 4],
    counts: [u32; 4],
    masked: bool,
    exception: u32,
    pending: bool,
) {
    assert_eq!(v, &[0xe0000c60, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(counts, [0; 4]);
    assert!(masked && exception == 0 && !pending);
    assert_eq!(a[2..4], [8, 1]);
    for address in &a[..2] {
        assert!((0x20000000..=0x2002dff8).contains(address) && address % 4 == 0);
    }
    assert_eq!(
        a[0].checked_add(12),
        Some(a[1]),
        "actual raw allocation stride"
    );
    assert!(
        a[0] % 8 == 4 || a[1] % 8 == 4,
        "observed missing fundamental alignment"
    );
}

#[test]
fn manufactured_heap_alignment_bug_witness_refuses() {
    let good = [0xe0000c60, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let allocation = [0x20001004, 0x20001010, 8, 1];
    heap_alignment_bug(&good, allocation, [0; 4], true, 0, false);
    heap_alignment_bug(
        &good,
        [0x20001000, 0x2000100c, 8, 1],
        [0; 4],
        true,
        0,
        false,
    );
    for index in 0..good.len() {
        let mut bad = good;
        bad[index] = if good[index] == 0 { 1 } else { 0 };
        assert!(std::panic::catch_unwind(|| heap_alignment_bug(
            &bad, allocation, [0; 4], true, 0, false
        ))
        .is_err());
    }
    for bad in [
        [0, allocation[1], 8, 1],
        [allocation[0], 0, 8, 1],
        [allocation[0], allocation[0], 8, 1],
        [allocation[1], allocation[0], 8, 1],
        [0x20001000, 0x20001010, 8, 1],
        [0x20001002, 0x2000100e, 8, 1],
        [0x1ffffffc, 0x20000008, 8, 1],
        [0x2002dff8, 0x2002e004, 8, 1],
        [0xfffffff8, 4, 8, 1],
        [allocation[0], allocation[1], 0, 1],
        [allocation[0], allocation[1], 4, 1],
        [allocation[0], allocation[1], 8, 0],
        [allocation[0], allocation[1], 8, 2],
    ] {
        assert!(std::panic::catch_unwind(|| heap_alignment_bug(
            &good, bad, [0; 4], true, 0, false
        ))
        .is_err());
    }
    for index in 0..4 {
        let mut counts = [0; 4];
        counts[index] = 1;
        assert!(std::panic::catch_unwind(|| heap_alignment_bug(
            &good, allocation, counts, true, 0, false
        ))
        .is_err());
    }
    for (masked, exception, pending) in [(false, 0, false), (true, 15, false), (true, 0, true)] {
        assert!(std::panic::catch_unwind(|| heap_alignment_bug(
            &good, allocation, [0; 4], masked, exception, pending
        ))
        .is_err());
    }
}

fn public_heap_terminal(mode: u32, v: &[u32], w: [u32; 4], counts: [u32; 4]) {
    assert!([6, 7].contains(&mode));
    assert_eq!(v.len(), NAMES.len());
    assert_eq!(counts, [0; 4]);
    assert_eq!(&v[2..], &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    if mode == 6 {
        assert_eq!(&v[..2], &[0x600d, 2]);
        assert_eq!(w, [641, 1, 0, 0]);
    } else {
        assert_eq!(&v[..2], &[0xe0000014, 1]);
        assert_eq!(&w[..2], &[0, 2]);
        assert!(w[2] != w[3]);
        assert!(w[2..]
            .iter()
            .all(|a| (0x20000000..0x2002e000).contains(a) && a % 8 == 0));
    }
}

#[test]
fn manufactured_public_heap_terminal_refuses() {
    for (mode, status, checks, w) in [
        (6, 0x600d, 2, [641, 1, 0, 0]),
        (7, 0xe0000014, 1, [0, 2, 0x20001008, 0x20001040]),
    ] {
        let good = [status, checks, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        public_heap_terminal(mode, &good, w, [0; 4]);
        for index in 0..good.len() {
            let mut bad = good;
            bad[index] = if good[index] == 0 { 1 } else { 0 };
            assert!(
                std::panic::catch_unwind(|| public_heap_terminal(mode, &bad, w, [0; 4])).is_err()
            );
        }
        for index in 0..w.len() {
            let mut bad = w;
            bad[index] = if w[index] == 0 { 1 } else { 0 };
            assert!(
                std::panic::catch_unwind(|| public_heap_terminal(mode, &good, bad, [0; 4]))
                    .is_err()
            );
        }
        for index in 0..4 {
            let mut bad = [0; 4];
            bad[index] = 1;
            assert!(
                std::panic::catch_unwind(|| public_heap_terminal(mode, &good, w, bad)).is_err()
            );
        }
    }
}

#[test]
#[ignore = "prospective real services; dedicated source-bound hosted execution required"]
fn actual_services_deliver_timer_event_to_real_waiting_fibers_and_refuse_closed_switch() {
    let directory = PathBuf::from(std::env::var("LABWIRED_SERVICES_FIXTURE_DIR").unwrap());
    let source = std::env::var("LABWIRED_SERVICES_SOURCE").unwrap();
    assert!(source.len() == 40 && source.bytes().all(|b| b.is_ascii_hexdigit()));
    for (mode, name) in [
        (0, "positive"),
        (1, "closed-switch"),
        (2, "clock-irq-disable"),
        (3, "clock-disable"),
        (4, "long-gap"),
        (5, "heap-alignment"),
        (6, "public-heap"),
        (7, "public-heap-oom"),
    ] {
        let folder = directory.join(name);
        let provenance: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("build-provenance.json")).unwrap())
                .unwrap();
        assert_eq!(provenance["source"].as_str(), Some(source.as_str()));
        assert_eq!(provenance["core_source"], CORE);
        assert_eq!(provenance["caller_mode"], mode);
        assert_eq!(provenance["cpu_hz"], 120000000);
        assert_eq!(provenance["runtime_admission"], false);
        let path = folder.join("control.elf");
        let bytes = std::fs::read(&path).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(provenance["elf_sha256"].as_str(), Some(hash.as_str()));
        let map_hash = format!(
            "{:x}",
            Sha256::digest(std::fs::read(folder.join("control.map")).unwrap())
        );
        assert_eq!(provenance["map_sha256"].as_str(), Some(map_hash.as_str()));
        let elf = goblin::elf::Elf::parse(&bytes).unwrap();
        let symbol = |name| {
            let matches: Vec<_> = elf
                .syms
                .iter()
                .filter(|s| s.st_shndx != 0 && elf.strtab.get_at(s.st_name) == Some(name))
                .collect();
            assert_eq!(matches.len(), 1, "unique defined symbol {name}");
            matches[0].st_value
        };
        let addresses: Vec<_> = NAMES.iter().map(|name| symbol(name)).collect();
        let clock_addresses = if (2..=4).contains(&mode) {
            Some(
                [
                    "runtime_clock_stage",
                    "runtime_clock_pending_seen",
                    "runtime_clock_before",
                    "runtime_clock_after",
                ]
                .map(symbol),
            )
        } else {
            None
        };
        let mut distinct = addresses.clone();
        let public_heap_addresses = (mode >= 6).then(|| {
            [
                "runtime_public_heap_checks",
                "runtime_public_heap_stage",
                "runtime_public_heap_old",
                "runtime_public_heap_guard",
            ]
            .map(symbol)
        });
        let alignment_addresses = (mode == 5).then(|| {
            [
                "runtime_alignment_first",
                "runtime_alignment_second",
                "runtime_alignment_size",
                "runtime_alignment_stage",
            ]
            .map(symbol)
        });
        let timer_addresses = (mode == 4).then(|| {
            [
                "runtime_timer_before",
                "runtime_timer_after",
                "runtime_ms_before",
                "runtime_ms_after",
            ]
            .map(symbol)
        });
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), NAMES.len());
        let bss = symbol("_bss_start")..symbol("_bss_end");
        let data = symbol("_data_start")..symbol("_data_end");
        assert!(data.start >= 0x20000000 && data.end <= bss.start && bss.end < 0x2002e000);
        assert!(addresses.iter().all(|a| bss.contains(a) && a % 4 == 0));
        if let Some(extra) = public_heap_addresses {
            assert!(extra.iter().all(|a| bss.contains(a) && a % 4 == 0));
            let mut all = addresses.clone();
            all.extend(extra);
            all.sort_unstable();
            all.dedup();
            assert_eq!(all.len(), NAMES.len() + 4);
        }
        if let Some(extra) = alignment_addresses {
            assert!(extra.iter().all(|a| bss.contains(a) && a % 4 == 0));
            let mut all = addresses.clone();
            all.extend(extra);
            all.sort_unstable();
            all.dedup();
            assert_eq!(all.len(), NAMES.len() + 4);
        }
        if let Some(extra) = clock_addresses {
            assert!(extra.iter().all(|a| bss.contains(a) && a % 4 == 0));
            let mut all = addresses.clone();
            all.extend(extra);
            if let Some(timer) = timer_addresses {
                assert!(timer.iter().all(|a| bss.contains(a) && a % 4 == 0));
                all.extend(timer);
            }
            all.sort_unstable();
            all.dedup();
            assert_eq!(all.len(), NAMES.len() + 4 + if mode == 4 { 4 } else { 0 });
        }
        assert_eq!(symbol("runtime_vectors"), 0);
        assert_eq!(symbol("runtime_vectors_end"), 1024);
        let calls = Arc::new(Calls {
            swap: (symbol("swap_context") & !1) as u32,
            restore: (symbol("restore_register_context") & !1) as u32,
            swaps: AtomicU32::new(0),
            restores: AtomicU32::new(0),
        });
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51.yaml")).unwrap();
        assert_eq!(chip.cpu_hz, 120000000);
        let manifest: SystemManifest = serde_yaml::from_str(
            "name: real-services\nchip: ../chips/atsamd51.yaml\nexternal_devices: []\n",
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine.add_observer(calls.clone());
        machine
            .load_firmware(&labwired_loader::load_elf(&path).unwrap())
            .unwrap();
        assert_eq!(machine.bus.read_u32(0).unwrap(), 0x20030000);
        for vector in 1..256 {
            let word = machine.bus.read_u32(vector * 4).unwrap();
            let target = match vector {
                1 => "reset_entry",
                15 => "runtime_systick",
                _ => "runtime_fault",
            };
            assert_eq!(word & 1, 1);
            assert_eq!(u64::from(word & !1), symbol(target) & !1);
        }
        let load = symbol("_data_load");
        let expected_data: Vec<_> = (0..data.end - data.start)
            .step_by(4)
            .map(|offset| machine.bus.read_u32(load + offset).unwrap())
            .collect();
        for address in bss.clone().step_by(4) {
            machine.bus.write_u32(address, 0xa5a5a5a5).unwrap();
        }
        for address in data.clone().step_by(4) {
            machine.bus.write_u32(address, 0x5a5a5a5a).unwrap();
        }
        let entry = symbol("runtime_probe") & !1;
        let mut startup = false;
        let mut previous_exception = 0;
        let mut entries = 0;
        let mut returns = 0;
        let mut final_step = None;
        let mut clock_stage = 0;
        let mut pending_under_mask = false;
        let mut disabled_without_event = false;
        // The existing eight cases retain their original bound. An80ms guest
        // necessarily needs its separately named larger execution budget.
        let step_bound = if mode == 4 { 12_000_000 } else { 4_000_000 };
        for step in 1..=step_bound {
            machine.step().expect("real services guest must not fault");
            if u64::from(machine.cpu.pc & !1) == entry && !startup {
                for address in bss.clone().step_by(4) {
                    assert_eq!(machine.bus.read_u32(address).unwrap(), 0);
                }
                for (address, expected) in data.clone().step_by(4).zip(&expected_data) {
                    assert_eq!(machine.bus.read_u32(address).unwrap(), *expected);
                }
                assert!(machine.cpu.primask && machine.cpu.active_exception == 0);
                startup = true;
            }
            let exception = machine.cpu.active_exception;
            assert!(
                [0, 15].contains(&exception),
                "unexpected architectural exception"
            );
            if exception == 15 && previous_exception == 0 {
                entries += 1;
            }
            if exception == 0 && previous_exception == 15 {
                returns += 1;
            }
            previous_exception = exception;
            if !startup {
                continue;
            }
            let v: Vec<_> = addresses
                .iter()
                .map(|a| machine.bus.read_u32(*a).unwrap())
                .collect();
            let swaps = calls.swaps.load(Ordering::Relaxed);
            let restores = calls.restores.load(Ordering::Relaxed);
            if mode >= 6 {
                let w = public_heap_addresses
                    .unwrap()
                    .map(|address| machine.bus.read_u32(address).unwrap());
                let pending = machine.cpu.pending_exceptions[0] & (1u64 << 15) != 0;
                if w[1] != clock_stage {
                    assert!(machine.cpu.primask && exception == 0 && !pending);
                    assert_eq!(clock_stage, 0);
                    assert_eq!(w[1], if mode == 6 { 1 } else { 2 });
                    clock_stage = w[1];
                    println!("SAMD_PUBLIC_HEAP_BOUNDARY mode={name} stage={} step={step} checks={} old={} guard={} mask=1 exception=0 pending=false irq_entries={entries} irq_returns={returns} actual_swaps={swaps} actual_restores={restores}", w[1], w[0], w[2], w[3]);
                }
                if [0, 0x100].contains(&v[0]) {
                    continue;
                }
                assert!(machine.cpu.primask && exception == 0 && !pending);
                public_heap_terminal(mode, &v, w, [entries, returns, swaps, restores]);
                assert_eq!(clock_stage, if mode == 6 { 1 } else { 2 });
                assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
                assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
                if mode == 7 {
                    let heap_start = u32::try_from(symbol("_heap_start")).unwrap();
                    let heap_end = 0x2002e000;
                    let extent = heap_end - heap_start;
                    assert!(extent > 0x20000 + 32 + 2 * 19 && extent < 2 * 0x20000);
                    for (pointer, requested) in [(w[2], 32), (w[3], 0x20000)] {
                        assert!(
                            pointer >= heap_start + 12
                                && pointer
                                    .checked_add(requested)
                                    .is_some_and(|end| end <= heap_end)
                        );
                        let raw = machine.bus.read_u32(u64::from(pointer - 8)).unwrap();
                        assert_eq!(
                            machine.bus.read_u32(u64::from(pointer - 4)).unwrap(),
                            requested
                        );
                        assert!(
                            raw >= heap_start + 4
                                && raw % 4 == 0
                                && raw <= pointer - 8
                                && pointer - raw <= 15
                        );
                        let block = machine.bus.read_u32(u64::from(raw - 4)).unwrap();
                        assert_eq!(
                            block & 0x80000000,
                            0,
                            "actual old/guard allocation remains live"
                        );
                        assert!(block > 1);
                        let capacity = (block - 1).checked_mul(4).unwrap();
                        assert!(raw.checked_add(capacity).is_some_and(|end| end <= heap_end));
                        assert!(pointer.checked_add(requested).unwrap() <= raw + capacity);
                    }
                    for offset in 0..32 {
                        assert_eq!(machine.bus.read_u8(u64::from(w[2] + offset)).unwrap(), 0x69);
                    }
                    println!("SAMD_PUBLIC_HEAP_OOM_BINDING heap_start={heap_start} heap_end={heap_end} heap_extent={extent} old={} old_size=32 old_live=true old_payload_69=true guard={} guard_size=131072 guard_live=true", w[2], w[3]);
                }
                println!("SAMD_PUBLIC_HEAP_RESULT mode={name} source={source} core={CORE} elf_sha256={hash} steps={step} status={:#x} stage={} checks={} old={} guard={} heap_reused=1 irq_entries=0 irq_returns=0 actual_swaps=0 actual_restores=0 final_mask=1 final_exception=0 pending=false step_bound={step_bound} bss_clear=true data_copy=true vectors=true runtime_admission=false", v[0], w[1], w[0], w[2], w[3]);
                final_step = Some(step);
                break;
            }
            if mode == 5 {
                let a = alignment_addresses
                    .unwrap()
                    .map(|address| machine.bus.read_u32(address).unwrap());
                let pending = machine.cpu.pending_exceptions[0] & (1u64 << 15) != 0;
                if a[3] != clock_stage {
                    assert_eq!(a[3], 1);
                    assert_eq!(clock_stage, 0);
                    clock_stage = a[3];
                    println!("SAMD_HEAP_ALIGNMENT_BOUNDARY stage=1 step={step} first={} second={} size={} mask={} exception={exception} pending={pending} irq_entries={entries} irq_returns={returns} actual_swaps={swaps} actual_restores={restores}", a[0], a[1], a[2], u32::from(machine.cpu.primask));
                }
                if [0, 0x100].contains(&v[0]) {
                    continue;
                }
                heap_alignment_bug(
                    &v,
                    a,
                    [entries, returns, swaps, restores],
                    machine.cpu.primask,
                    exception,
                    pending,
                );
                assert_eq!(clock_stage, 1);
                assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
                assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
                println!("SAMD_HEAP_ALIGNMENT_BUG source={source} core={CORE} elf_sha256={hash} steps={step} status=0xe0000c60 stage=1 first={} second={} size=8 stride=12 missing_alignment=true checks=1 heap_reused=1 irq_entries=0 irq_returns=0 actual_swaps=0 actual_restores=0 final_mask=1 final_exception=0 pending=false step_bound={step_bound} bss_clear=true data_copy=true vectors=true runtime_admission=false", a[0], a[1]);
                final_step = Some(step);
                break;
            }
            if mode == 4 {
                let c = clock_addresses
                    .unwrap()
                    .map(|a| machine.bus.read_u32(a).unwrap());
                let t = timer_addresses
                    .unwrap()
                    .map(|a| machine.bus.read_u32(a).unwrap());
                let pending = machine.cpu.pending_exceptions[0] & (1u64 << 15) != 0;
                if pending && machine.cpu.primask && [1, 2].contains(&c[0]) {
                    if !pending_under_mask {
                        println!("SAMD_SERVICE_LONG_FIXED_BOUNDARY kind=pending stage={} step={step} mask=1 exception={exception} pending=true event_calls={} irq_entries={entries} irq_returns={returns}", c[0], v[6]);
                    }
                    pending_under_mask = true;
                }
                if c[0] != clock_stage {
                    assert_eq!(c[0], clock_stage + 1);
                    clock_stage = c[0];
                    println!("SAMD_SERVICE_LONG_FIXED_BOUNDARY kind=stage stage={clock_stage} step={step} mask={} exception={exception} pending={pending} event_calls={} irq_entries={entries} irq_returns={returns} counter_before={} counter_after={} timer_before={} timer_after={} ms_before={} ms_after={}", u32::from(machine.cpu.primask), v[6], c[2], c[3], t[0], t[1], t[2], t[3]);
                    if clock_stage == 2 {
                        assert!(pending && machine.cpu.primask && exception == 0);
                        assert_eq!((entries, returns, v[6]), (0, 0, 0));
                    }
                }
                if [0, 0x100].contains(&v[0]) {
                    continue;
                }
                // Separately named positive qualification. The old exact bug
                // diagnostic remains preserved on its own source and branch.
                assert_eq!(v[0..7], [0x600d, 3, 1, 0, 0, 1, 1]);
                assert_eq!(v[7..11], [15, 0, 0, 0]);
                assert!(v[11] != 0 && v[12] == 0);
                assert_eq!((entries, returns, swaps, restores), (1, 1, 0, 0));
                assert_eq!(c[0..2], [4, 1]);
                assert!(pending_under_mask && !pending && machine.cpu.primask && exception == 0);
                assert!(c[2] > 0 && t[0] > 0 && c[3] >= c[2] && t[1] >= t[0]);
                let low_elapsed = c[3] - c[2];
                let timer_elapsed = t[1] - t[0];
                assert!((80_000..81_000).contains(&low_elapsed));
                assert!(timer_elapsed >= 80_000 && timer_elapsed <= low_elapsed + 1000);
                assert!(t[3] >= t[2]);
                let ms_elapsed = t[3] - t[2];
                assert!((80..=82).contains(&ms_elapsed));
                assert!(ms_elapsed * 1000 + 1000 >= timer_elapsed);
                assert!(ms_elapsed * 1000 <= timer_elapsed + 1000);
                assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
                assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
                println!("SAMD_SERVICE_LONG_FIXED source={source} core={CORE} elf_sha256={hash} steps={step} status=0x600d stage=4 low_before={} low_after={} timer_before={} timer_after={} ms_before={} ms_after={} pending_under_mask=true source_countflag=true event_calls=1 irq_entries=1 irq_returns=1 actual_swaps=0 actual_restores=0 final_mask=1 final_exception=0 step_bound={step_bound} bss_clear=true data_copy=true vectors=true", c[2], c[3], t[0], t[1], t[2], t[3]);
                final_step = Some(step);
                break;
            }
            if let Some(clock_addresses) = clock_addresses {
                let c = clock_addresses.map(|a| machine.bus.read_u32(a).unwrap());
                let pending = machine.cpu.pending_exceptions[0] & (1u64 << 15) != 0;
                if pending && machine.cpu.primask && [1, 2].contains(&c[0]) {
                    if !pending_under_mask {
                        println!("SAMD_SERVICE_CLOCK_BOUNDARY mode={name} kind=pending stage={} step={step} mask=1 exception={exception} pending=true event_calls={} irq_entries={entries} irq_returns={returns}", c[0], v[6]);
                    }
                    pending_under_mask = true;
                }
                if c[0] != clock_stage {
                    assert_eq!(c[0], clock_stage + 1, "actual ordered clock stages");
                    clock_stage = c[0];
                    println!("SAMD_SERVICE_CLOCK_BOUNDARY mode={name} kind=stage stage={clock_stage} step={step} mask={} exception={exception} pending={pending} event_calls={} irq_entries={entries} irq_returns={returns} counter_before={} counter_after={}", u32::from(machine.cpu.primask), v[6], c[2], c[3]);
                    if clock_stage == 2 {
                        assert!(pending && machine.cpu.primask && exception == 0);
                        assert_eq!(entries, 0);
                        assert_eq!(returns, 0);
                        assert_eq!(v[6], 0);
                    }
                    if clock_stage == 3 {
                        disabled_without_event = v[6] == 0;
                    }
                }
                if ![0, 0x100, 0x600d].contains(&v[0]) {
                    println!("SAMD_SERVICE_CLOCK_FAILURE mode={name} source={source} core={CORE} elf_sha256={hash} steps={step} status={:#x} stage={clock_stage} pending_seen={} event_calls={} irq_entries={entries} irq_returns={returns} counter_before={} counter_after={}", v[0], c[1], v[6], c[2], c[3]);
                    panic!("actual clock adversary failure {:#x}", v[0]);
                }
                if v[0] != 0x600d {
                    continue;
                }
                assert!(pending_under_mask && disabled_without_event);
                assert_eq!(clock_stage, 5);
                assert_eq!(c[1], 1);
                assert!(c[2] > 0);
                assert!(if mode == 2 { c[3] > c[2] } else { c[3] == c[2] });
                assert_eq!(v[0..7], [0x600d, 3, 1, 0, 0, 2, 1]);
                assert_eq!(v[7..11], [15, 0, 0, 0]);
                assert!(v[11] != 0 && v[12] == 0);
                assert_eq!((entries, returns, swaps, restores), (2, 2, 0, 0));
                println!("SAMD_SERVICE_CLOCK mode={name} source={source} core={CORE} elf_sha256={hash} steps={step} bss_clear=true data_copy=true vectors=true stages=5 pending_under_mask=true disabled_without_event=true counter_before={} counter_after={} event_calls=1 irq_entries=2 irq_returns=2 actual_swaps=0 actual_restores=0 final_mask=1 final_exception=0", c[2], c[3]);
            } else if mode == 0 {
                assert!(
                    [0, 0x100, 0x600d].contains(&v[0]),
                    "explicit services failure {:#x}",
                    v[0]
                );
                if v[0] != 0x600d {
                    continue;
                }
                positive(&v, entries, returns, swaps, restores);
            } else {
                assert!(
                    [0, 0x100, 0xe0000d10].contains(&v[0]),
                    "wrong refusal {:#x}",
                    v[0]
                );
                if v[0] != 0xe0000d10 {
                    continue;
                }
                assert_eq!(&v[0..5], &[0xe0000d10, 2, 1, 0, 0]);
                assert_eq!(&v[6..11], &[0, 0, 0, 0, 0]);
                assert_eq!(swaps, 0);
                assert_eq!(restores, 0);
                assert!(v[11] != 0 && v[12] == 0);
            }
            assert!(machine.cpu.primask && exception == 0);
            assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
            assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
            if mode < 2 {
                println!("SAMD_REAL_SERVICES mode={name} source={source} core={CORE} elf_sha256={hash} steps={step} bss_clear=true data_copy=true vectors=true witnesses={v:?} irq_entries={entries} irq_returns={returns} actual_swaps={swaps} actual_restores={restores} final_mask=1 final_exception=0");
            }
            final_step = Some(step);
            break;
        }
        assert!(
            startup && final_step.is_some(),
            "timeout is not real service qualification"
        );
    }
}
