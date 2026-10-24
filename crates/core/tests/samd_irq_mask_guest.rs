// SPDX-License-Identifier: MIT
//! Authored software-pended IRQ0 only; not peripheral/DMA IRQ or runtime admission.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, system::cortex_m::configure_cortex_m, Bus, Machine};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[test]
#[ignore = "hosted source-built IRQ0 fixture; run the dedicated qualification workflow"]
fn authored_irq_waits_for_outer_restore_and_detects_early_delivery() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = PathBuf::from(std::env::var("LABWIRED_IRQ_FIXTURE_DIR").unwrap());
    for (case, expected, stage) in [("positive", 0x600d, 2), ("restore-enabled", 6, 1)] {
        let path = directory.join(case).join("control.elf");
        let bytes = std::fs::read(&path).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let elf = goblin::elf::Elf::parse(&bytes).expect("owned unstripped ELF");
        let symbol = |name| {
            // The general loader helper intentionally rejects zero addresses.
            // This fixture has a defined vector-table symbol at flash address0.
            let matches: Vec<_> = elf
                .syms
                .iter()
                .filter(|sym| sym.st_shndx != 0 && elf.strtab.get_at(sym.st_name) == Some(name))
                .collect();
            assert_eq!(matches.len(), 1, "unique defined guest symbol {name}");
            assert!(matches[0].st_value <= u32::MAX as u64);
            matches[0].st_value
        };
        let status = symbol("control_status");
        let witnesses = [
            "irq_count",
            "irq_stage_seen",
            "irq_mask_seen",
            "irq_exception_seen",
        ]
        .map(symbol);
        for address in std::iter::once(&status).chain(witnesses.iter()) {
            assert!((0x2000_0000..0x2003_0000).contains(address));
            assert_eq!(address % 4, 0);
        }
        let mut slots: Vec<_> = std::iter::once(status).chain(witnesses).collect();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), 5, "distinct status and handler witnesses");
        assert_eq!(symbol("authored_vectors"), 0);
        assert_eq!(symbol("authored_vectors_end"), 1024);
        let irq_handler = symbol("authored_irq0") as u32;
        let trap_handler = symbol("unexpected_exception") as u32;
        let reset_handler = symbol("reset_entry") as u32;
        let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51.yaml")).unwrap();
        let manifest: SystemManifest = serde_yaml::from_str(
            "name: authored-irq-control\nchip: ../chips/atsamd51.yaml\nexternal_devices: []\n",
        )
        .unwrap();
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        machine
            .load_firmware(&labwired_loader::load_elf(&path).unwrap())
            .unwrap();
        // Validate every vector word, including reserved slots and trap extent.
        assert_eq!(machine.bus.read_u32(0).unwrap(), 0x2003_0000);
        for vector in 1..256 {
            let word = machine.bus.read_u32(vector * 4).unwrap();
            if [7, 8, 9, 10, 13].contains(&vector) {
                assert_eq!(word, 0);
            } else {
                let target = match vector {
                    1 => reset_handler,
                    16 => irq_handler,
                    _ => trap_handler,
                };
                assert_eq!(word & 1, 1);
                assert_eq!(word & !1, target & !1);
            }
        }
        // Host changes only authored BSS witnesses, never IRQ pending state.
        for address in std::iter::once(&status).chain(witnesses.iter()) {
            machine.bus.write_u32(*address, 0xa5a5_a5a5).unwrap();
        }
        let mut saw_zero = false;
        let mut saw_entry = false;
        let mut saw_irq = false;
        let mut saw_return = false;
        let mut irq_entries = 0;
        let mut previous_exception = 0;
        let mut reached = false;
        let mut steps = 0;
        for step in 1..=100_000 {
            machine.step().expect("guest instruction must not fault");
            let observed = machine.bus.read_u32(status).unwrap();
            let values = witnesses.map(|address| machine.bus.read_u32(address).unwrap());
            saw_zero |= observed == 0 && values == [0, 0, 0, 0];
            saw_entry |= observed == 0x100;
            saw_irq |= machine.cpu.active_exception == 16;
            if machine.cpu.active_exception == 16 && previous_exception != 16 {
                irq_entries += 1;
            }
            saw_return |= previous_exception == 16 && machine.cpu.active_exception == 0;
            previous_exception = machine.cpu.active_exception;
            if observed != 0 && observed != 0x100 && observed != 0xa5a5_a5a5 {
                assert_eq!(observed, expected, "first guest result for {case}");
                assert_eq!(values, [1, stage, 0, 16], "actual handler witnesses");
                assert_eq!(machine.cpu.active_exception, 0, "returned to thread");
                if case == "positive" {
                    assert!(machine.cpu.primask, "positive cleanup remains masked");
                }
                reached = true;
                steps = step;
                break;
            }
        }
        assert!(saw_zero && saw_entry && saw_irq && saw_return && reached);
        assert_eq!(irq_entries, 1, "exactly one observed IRQ0 entry");
        println!("SAMD_IRQ_MASK case={case} elf_sha256={hash} status={expected:#x} steps={steps} bss_clear=true entry=true vectors=true irq_count=1 stage={stage} mask=0 exception=16 irq_entry=true irq_return=true");
    }
}
