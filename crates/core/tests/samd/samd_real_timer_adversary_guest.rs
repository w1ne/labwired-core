// SPDX-License-Identifier: MIT
//! Prospective actual owned-adapter mutants; no negative qualification yet.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn expected(case: &str) -> ([u32; 9], u32, u32) {
    match case {
        "unconditional_restore" => ([0xb02, 0, 0, 0, 0, 0, 0, 0, 1], 0, 0),
        "early_restore" => ([0xa14, 0, 1, 0, 0, 16, 0, 1, 1], 16, 1),
        "unsupported_event" => ([0xa07, 0, 0, 0, 0, 0, 0, 0, 0], 0, 0),
        _ => panic!("unknown authored mutant"),
    }
}

fn require_refusal(case: &str, values: [u32; 9], masked: bool, exception: u32, entries: u32) {
    let (witnesses, context, observed_entries) = expected(case);
    assert_eq!(values, witnesses, "exact authored refusal witnesses");
    assert!(masked, "refusal remains masked");
    assert_eq!(exception, context, "exact refusal exception context");
    assert_eq!(entries, observed_entries, "actual exception entry count");
}

fn require_terminal_pc(pc: u64, previous: Option<u64>, extent: &std::ops::Range<u64>) {
    assert!(
        extent.contains(&pc),
        "refusal may not escape its actual reject function"
    );
    if let Some(previous) = previous {
        assert_ne!(previous, pc, "a stopped CPU is not terminal execution");
    }
}

#[test]
fn manufactured_terminal_escape_and_stopped_pc_refuse() {
    require_terminal_pc(0x1002, Some(0x1000), &(0x1000..0x1010));
    for (pc, previous) in [
        (0x1010, Some(0x1000)),
        (0x0ffe, None),
        (0x1000, Some(0x1000)),
    ] {
        assert!(
            std::panic::catch_unwind(|| require_terminal_pc(pc, previous, &(0x1000..0x1010)))
                .is_err()
        );
    }
}

#[test]
fn manufactured_three_refusal_witnesses_accept() {
    for case in [
        "unconditional_restore",
        "early_restore",
        "unsupported_event",
    ] {
        let (values, exception, entries) = expected(case);
        require_refusal(case, values, true, exception, entries);
    }
}

#[test]
fn manufactured_wrong_refusal_witness_or_context_refuses() {
    for case in [
        "unconditional_restore",
        "early_restore",
        "unsupported_event",
    ] {
        let (values, exception, entries) = expected(case);
        for index in 0..9 {
            let mut corrupt = values;
            corrupt[index] ^= 1;
            assert!(std::panic::catch_unwind(|| require_refusal(
                case, corrupt, true, exception, entries
            ))
            .is_err());
        }
        for (masked, context, count) in [
            (false, exception, entries),
            (true, exception ^ 16, entries),
            (true, exception, entries + 1),
        ] {
            assert!(std::panic::catch_unwind(|| require_refusal(
                case, values, masked, context, count
            ))
            .is_err());
        }
    }
}

#[test]
#[ignore = "prospective Timer adapter mutants; bound hosted build and dedicated guest qualification required"]
fn actual_timer_adversaries_reject_exactly_and_remain_terminal() {
    let directory = PathBuf::from(std::env::var("LABWIRED_TIMER_MUTANT_DIR").unwrap());
    for case in [
        "unconditional_restore",
        "early_restore",
        "unsupported_event",
    ] {
        let fixture = directory.join(case);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.join("mutation.json")).unwrap()).unwrap();
        assert_eq!(manifest["case"], case);
        assert_eq!(
            manifest["samd_commit"],
            "d55214588780d737640536b0a58707cd6d69cfce"
        );
        assert_eq!(manifest["guest_executed"], false);
        assert_eq!(manifest["runtime_admitted"], false);
        let path = fixture.join("control.elf");
        let bytes = std::fs::read(&path).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(manifest["elf"]["sha256"].as_str(), Some(hash.as_str()));
        assert_eq!(manifest["elf"]["bytes"].as_u64(), Some(bytes.len() as u64));
        let elf = goblin::elf::Elf::parse(&bytes).unwrap();
        let symbol = |name| {
            let matches: Vec<_> = elf
                .syms
                .iter()
                .filter(|sym| sym.st_shndx != 0 && elf.strtab.get_at(sym.st_name) == Some(name))
                .collect();
            assert_eq!(matches.len(), 1, "unique defined symbol {name}");
            (matches[0].st_value, matches[0].st_size)
        };
        let names = [
            "control_status",
            "bw_simulation_irq_fault",
            "caller_checks",
            "irq_count",
            "irq_mask_seen",
            "irq_exception_seen",
            "irq_closed_seen",
            "compare_count",
            "component_count",
        ];
        let addresses = names.map(|name| symbol(name).0);
        let mut distinct = addresses.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 9);
        let bss = symbol("_bss_start").0..symbol("_bss_end").0;
        let data = symbol("_data_start").0..symbol("_data_end").0;
        for extent in [&bss, &data] {
            assert!(
                extent.start >= 0x2000_0000
                    && extent.end <= 0x2002_f000
                    && extent.start < extent.end
            );
            assert_eq!(extent.start % 4, 0);
            assert_eq!(extent.end % 4, 0);
        }
        assert!(data.end <= bss.start || bss.end <= data.start);
        assert!(addresses
            .iter()
            .all(|address| bss.contains(address) && address % 4 == 0));
        assert_eq!(symbol("authored_vectors").0, 0);
        assert_eq!(symbol("authored_vectors_end").0, 1024);
        let (reject_start, reject_size) = symbol("_ZL6rejectj");
        let reject_start = reject_start & !1;
        assert!(reject_size > 0 && reject_start.checked_add(reject_size).unwrap() <= 0x40000);
        let terminal_code = reject_start..reject_start + reject_size;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51.yaml")).unwrap();
        let system: SystemManifest = serde_yaml::from_str(
            "name: timer-mutant\nchip: ../chips/atsamd51.yaml\nexternal_devices: []\n",
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &system).unwrap();
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine
            .load_firmware(&labwired_loader::load_elf(&path).unwrap())
            .unwrap();
        assert_eq!(machine.bus.read_u32(0).unwrap(), 0x2003_0000);
        for vector in 1..256 {
            let word = machine.bus.read_u32(vector * 4).unwrap();
            if [7, 8, 9, 10, 13].contains(&vector) {
                assert_eq!(word, 0);
            } else {
                let name = match vector {
                    1 => "reset_entry",
                    16 => "authored_irq0",
                    _ => "unexpected_exception",
                };
                assert_eq!(word & 1, 1);
                assert_eq!(u64::from(word & !1), symbol(name).0 & !1);
            }
        }
        let load = symbol("_data_load").0;
        assert!(load + data.end - data.start <= 0x40000);
        let expected_data: Vec<_> = (0..data.end - data.start)
            .step_by(4)
            .map(|offset| machine.bus.read_u32(load + offset).unwrap())
            .collect();
        for address in bss.clone().step_by(4) {
            machine.bus.write_u32(address, 0xa5a5_a5a5).unwrap();
        }
        for address in data.clone().step_by(4) {
            machine.bus.write_u32(address, 0x5a5a_5a5a).unwrap();
        }
        let entry = symbol("irq_mask_probe").0 & !1;
        let mut startup = false;
        let mut entries = 0;
        let mut previous_exception = 0;
        let mut terminal = 0;
        let mut previous_terminal_pc = None;
        let mut final_step = None;
        let (required, _, _) = expected(case);
        for step in 1..=250_000 {
            machine
                .step()
                .expect("engine failure is not authored rejection");
            let pc = u64::from(machine.cpu.pc & !1);
            if pc == entry && !startup {
                for address in bss.clone().step_by(4) {
                    assert_eq!(machine.bus.read_u32(address).unwrap(), 0);
                }
                for (address, value) in data.clone().step_by(4).zip(&expected_data) {
                    assert_eq!(machine.bus.read_u32(address).unwrap(), *value);
                }
                assert!(machine.cpu.primask && machine.cpu.active_exception == 0);
                startup = true;
            }
            let exception = machine.cpu.active_exception;
            if exception == 16 && previous_exception != 16 {
                entries += 1;
            }
            previous_exception = exception;
            if !startup {
                continue;
            }
            let values = addresses.map(|address| machine.bus.read_u32(address).unwrap());
            if terminal != 0 {
                assert_eq!(values[0], required[0], "terminal status may not regress");
            }
            assert_eq!(
                values[1], 0,
                "unexpected paired fault is not the required rejection"
            );
            assert!(
                [0, 0x100, required[0]].contains(&values[0]),
                "wrong failure or success is not rejection"
            );
            if values[0] == required[0] {
                require_refusal(case, values, machine.cpu.primask, exception, entries);
                require_terminal_pc(pc, previous_terminal_pc, &terminal_code);
                previous_terminal_pc = Some(pc);
                terminal += 1;
                if terminal == 64 {
                    final_step = Some(step);
                    break;
                }
            }
        }
        let steps = final_step.expect("timeout is not successful guest rejection");
        assert!(startup);
        let (_, exception, entries) = expected(case);
        println!("SAMD_TIMER_REFUSAL case={case} elf_sha256={hash} steps={steps} terminal=64 status={:#x} fault=0 mask=1 exception={exception} irq_entries={entries} milestones={} startup=true", required[0], required[2]);
    }
}
