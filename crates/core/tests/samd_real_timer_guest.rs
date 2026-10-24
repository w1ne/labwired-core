// SPDX-License-Identifier: MIT
//! Prospective real Timer caller; explicit diagnostic adapters, not full runtime.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn require_complete(
    values: [u32; 9],
    entries: u32,
    milestones: u32,
    returns: u32,
    masked: bool,
    exception: u32,
) {
    assert_eq!(
        values,
        [0x600d, 0, 6, 2, 0, 16, 1, 3, 0],
        "exact Timer witnesses"
    );
    assert_eq!(entries, 2, "two actual exception entries");
    assert_eq!(milestones, 6, "observe each ordered caller milestone");
    assert_eq!(returns, 2, "two actual exception returns");
    assert!(masked && exception == 0, "masked thread return required");
}

#[test]
fn manufactured_complete_timer_witness_accepts() {
    require_complete([0x600d, 0, 6, 2, 0, 16, 1, 3, 0], 2, 6, 2, true, 0);
}

#[test]
fn manufactured_missing_timer_witness_refuses() {
    for index in 0..9 {
        let mut values = [0x600d, 0, 6, 2, 0, 16, 1, 3, 0];
        values[index] ^= 1;
        assert!(std::panic::catch_unwind(|| require_complete(values, 2, 6, 2, true, 0)).is_err());
    }
}

#[test]
fn manufactured_missing_execution_or_progress_refuses() {
    for (entries, milestones, returns, masked, exception) in [
        (1, 6, 2, true, 0),
        (2, 5, 2, true, 0),
        (2, 6, 1, true, 0),
        (2, 6, 2, false, 0),
        (2, 6, 2, true, 16),
    ] {
        assert!(std::panic::catch_unwind(|| require_complete(
            [0x600d, 0, 6, 2, 0, 16, 1, 3, 0],
            entries,
            milestones,
            returns,
            masked,
            exception
        ))
        .is_err());
    }
}

#[test]
#[ignore = "prospective actual Timer caller; qualified source and dedicated hosted execution required"]
fn actual_timer_nested_callers_restore_and_deliver_two_irqs() {
    let directory = PathBuf::from(std::env::var("LABWIRED_TIMER_FIXTURE_DIR").unwrap());
    let qualified = std::env::var("LABWIRED_TIMER_QUALIFIED_SOURCE").unwrap();
    assert!(qualified.len() == 40 && qualified.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("build-provenance.json")).unwrap())
            .unwrap();
    assert_eq!(
        provenance["source"]["samd_commit"].as_str(),
        Some(qualified.as_str())
    );
    assert_eq!(
        provenance["source"]["core_commit"],
        "2ed19d64ec3359143409a4b0a9758b4aeef3ee7d"
    );
    assert_eq!(provenance["guest_executed"], false);
    assert_eq!(provenance["runtime_admitted"], false);
    let path = directory.join("control.elf");
    let bytes = std::fs::read(&path).unwrap();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(provenance["elf"]["sha256"].as_str(), Some(hash.as_str()));
    assert_eq!(
        provenance["elf"]["bytes"].as_u64(),
        Some(bytes.len() as u64)
    );
    let elf = goblin::elf::Elf::parse(&bytes).unwrap();
    let symbol = |name| {
        let matches: Vec<_> = elf
            .syms
            .iter()
            .filter(|sym| sym.st_shndx != 0 && elf.strtab.get_at(sym.st_name) == Some(name))
            .collect();
        assert_eq!(matches.len(), 1, "unique defined symbol {name}");
        matches[0].st_value
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
    let addresses = names.map(symbol);
    let mut distinct = addresses.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 9);
    let bss = symbol("_bss_start")..symbol("_bss_end");
    let data = symbol("_data_start")..symbol("_data_end");
    for extent in [&bss, &data] {
        assert!(
            extent.start >= 0x2000_0000 && extent.end <= 0x2002_f000 && extent.start < extent.end
        );
        assert_eq!(extent.start % 4, 0);
        assert_eq!(extent.end % 4, 0);
    }
    assert!(data.end <= bss.start || bss.end <= data.start);
    assert!(addresses
        .iter()
        .all(|address| bss.contains(address) && address % 4 == 0));
    assert_eq!(symbol("authored_vectors"), 0);
    assert_eq!(symbol("authored_vectors_end"), 1024);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: real-timer-caller\nchip: ../chips/atsamd51.yaml\nexternal_devices: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
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
            assert_eq!(u64::from(word & !1), symbol(name) & !1);
        }
    }
    let load = symbol("_data_load");
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
    let entry = symbol("irq_mask_probe") & !1;
    let mut startup = false;
    let mut milestones = 0;
    let mut irq_entries = 0;
    let mut previous_exception = 0;
    let mut returns = 0;
    let mut reached = false;
    let mut final_step = 0;
    for step in 1..=250_000 {
        machine.step().expect("actual caller guest must not fault");
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
        if exception == 16 && previous_exception != 16 {
            irq_entries += 1;
        }
        if previous_exception == 16 && exception == 0 {
            returns += 1;
        }
        previous_exception = exception;
        if !startup {
            continue;
        }
        let values = addresses.map(|address| machine.bus.read_u32(address).unwrap());
        if values[2] != milestones {
            assert_eq!(values[2], milestones + 1, "each ordered caller milestone");
            milestones = values[2];
        }
        assert_eq!(values[1], 0, "unexpected paired fault");
        assert!(
            [0, 0x100, 0x600d].contains(&values[0]),
            "explicit guest failure {:#x}",
            values[0]
        );
        if values[0] == 0x600d {
            require_complete(
                values,
                irq_entries,
                milestones,
                returns,
                machine.cpu.primask,
                exception,
            );
            final_step = step;
            reached = true;
            break;
        }
    }
    assert!(startup && reached, "timeout is not caller qualification");
    println!("SAMD_REAL_TIMER elf_sha256={hash} source={qualified} steps={final_step} bss_clear=true data_copy=true vectors=true milestones=6 irq_entries=2 irq_returns=2 handler_mask=0 handler_exception=16 paired_closed=true fault=0 status=0x600d final_mask=1 final_exception=0");
}
