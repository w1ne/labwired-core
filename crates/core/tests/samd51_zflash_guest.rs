// SPDX-License-Identifier: MIT
//! Actual separately built public ZFlash caller, never a synthetic completion.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, system::cortex_m::configure_cortex_m, AdvanceRequest, AdvanceStop, Bus, Machine,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
const NVM: u64 = 0x41004000;
const START: usize = 0x78000;
fn board() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    for spec in ["id: nvm-manual\ntype: samd51_nvmctrl_manual_storage\nbase_address: 0x41004000\nsize: 256B\n", "id: cmcc-control\ntype: samd51_cmcc_control\nbase_address: 0x41006000\nsize: 256B\n"] {chip.peripherals.push(serde_yaml::from_str(spec).unwrap());}
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}
#[test]
fn opt_in_factory_is_distinct_and_default_board_unchanged() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    let default = SystemBus::from_config(&chip, &manifest).unwrap();
    assert!(!default.models_flash_ops());
    assert!(default.read_u32(NVM + 8).is_err());
    let mut bad = chip.clone();
    bad.peripherals.push(serde_yaml::from_str("id: bad-nvm\ntype: samd51_nvmctrl_manual_storage\nbase_address: 0x41005000\nsize: 256B\n").unwrap());
    assert!(SystemBus::from_config(&bad, &manifest).is_err());
    let b = board();
    assert!(b.models_flash_ops());
    assert!(!b.has_pending_flash_op());
    assert_eq!(b.read_u32(NVM + 8).unwrap(), 0x60400);
    assert_eq!(b.read_u32(0x4100600c).unwrap(), 0);
}
#[test]
#[ignore = "requires explicitly source-bound hosted ARM ZFlash fixture; never a default runtime claim"]
fn actual_zflash_driver_program_erase_and_refusals() {
    let path = PathBuf::from(std::env::var("LABWIRED_ZFLASH_FIXTURE").unwrap());
    let provenance: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("provenance.json")).unwrap()).unwrap();
    let bytes = std::fs::read(path.join("control.elf")).unwrap();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(provenance["elf_sha256"], hash);
    assert_eq!(provenance["guest_executed"], false);
    assert_eq!(
        provenance["samd_source"],
        "62a3a408ac544a4e34bf0beb3932683a2c114e26"
    );
    assert_eq!(
        provenance["core_source"],
        "2ed19d64ec3359143409a4b0a9758b4aeef3ee7d"
    );
    assert_eq!(
        provenance["driver_compile_report"]["adaptation"]["original_sha256"],
        "5058b18a0b9d4305a04cbecda13a8d973d61d2024a0645e3107273d90b045dd8"
    );
    let elf = goblin::elf::Elf::parse(&bytes).unwrap();
    let symbol = |needle: &str| -> u64 {
        let found: Vec<_> = elf
            .syms
            .iter()
            .filter(|s| s.st_shndx != 0 && elf.strtab.get_at(s.st_name) == Some(needle))
            .collect();
        assert_eq!(found.len(), 1, "unique symbol {needle}");
        found[0].st_value
    };
    let witness = symbol("zflash_witness");
    let checkpoint = (symbol("zflash_checkpoint") & !1) as u32;
    let method = |needle: &str| -> u32 {
        let found: Vec<_> = elf
            .syms
            .iter()
            .filter(|s| {
                s.st_shndx != 0
                    && elf
                        .strtab
                        .get_at(s.st_name)
                        .is_some_and(|n| n.contains("ZFlash") && n.contains(needle))
            })
            .collect();
        assert_eq!(found.len(), 1, "unique real ZFlash method {needle}");
        (found[0].st_value & !1) as u32
    };
    let irq_helpers = [
        (symbol("target_disable_irq") & !1) as u32,
        (symbol("target_enable_irq") & !1) as u32,
    ];
    let methods = [
        method("totalSize"),
        method("pageSize"),
        method("erasePage"),
        method("writeBytes"),
    ];
    assert!(methods.iter().all(|p| *p >= 0x4000 && *p < START as u32));
    assert!(symbol("_program_load_end") <= START as u64);
    assert!(witness >= 0x20000000 && witness + 64 <= symbol("_bss_end"));
    let mutant = std::env::var("ZFLASH_MUTANT").unwrap_or_default();
    assert!(["", "omit-erase-commit", "omit-program-commit"].contains(&mutant.as_str()));
    for batched in [false, true] {
        if !mutant.is_empty() && batched {
            continue;
        }
        let mut bus = board();
        let (cpu, _) = configure_cortex_m(&mut bus);
        let mut m = Machine::new(cpu, bus);
        m.load_firmware(&labwired_loader::load_elf(&path.join("control.elf")).unwrap())
            .unwrap();
        // Declared initial flash state, not guest behavior: the test block is
        // dirty and its immediate neighbors have distinguishable retained bytes.
        m.bus.flash.data[START..START + 8192].fill(0xa5);
        m.bus.flash.data[START - 1] = 0x5a;
        m.bus.flash.data[START + 8192] = 0xc3;
        let mut irq_calls = [0u32; 2];
        let mut irq_returns = [0u32; 2];
        let mut active_irq: Option<(usize, u32, bool)> = None;
        let mut phase = 0u32;
        let mut calls = [0u32; 4];
        let mut returns = [0u32; 4];
        let mut active: Vec<(usize, u32)> = Vec::new();
        let mut fuel = 0u64;
        let mut detected = false;
        if batched {
            m.breakpoints.insert(checkpoint);
        }
        loop {
            if batched {
                let report = m.advance(AdvanceRequest::run(Some(50000))).unwrap();
                fuel += report.primary_steps;
                if m.bus.read_u32(witness).unwrap() == 0x600d {
                    break;
                }
                assert_eq!(report.stop, AdvanceStop::Breakpoint(checkpoint));
            }
            assert!(fuel < 500000, "unchanged finite new-guest bound");
            let pc = m.cpu.pc & !1;
            if !batched {
                if let Some((kind, lr, entry_masked)) = active_irq {
                    if pc == lr {
                        assert_eq!(m.cpu.primask, kind == 0, "actual CPS helper return mask");
                        assert_eq!(m.cpu.active_exception, 0);
                        irq_returns[kind] += 1;
                        active_irq = None;
                        println!(
                            "ZFLASH_IRQ_RETURN {}",
                            serde_json::json!({"kind":kind,"ordinal":irq_returns[kind],"masked":m.cpu.primask,"entry_masked":entry_masked,"return_pc":lr,"elf_sha256":hash})
                        );
                    }
                }
                if let Some(kind) = irq_helpers.iter().position(|p| *p == pc) {
                    assert!(active_irq.is_none());
                    assert_eq!(m.cpu.active_exception, 0);
                    assert!(active.last().is_some_and(|(method, _)| *method == 3));
                    assert_eq!(m.cpu.primask, kind == 1, "nonnested CPS helper entry mask");
                    irq_calls[kind] += 1;
                    active_irq = Some((kind, m.cpu.lr & !1, m.cpu.primask));
                }
                if let Some(&(kind, lr)) = active.last() {
                    if pc == lr {
                        active.pop();
                        returns[kind] += 1;
                        let expected = match kind {
                            0 => 0x80000,
                            1 => 8192,
                            2 => 0,
                            3 => [0, 0, u32::MAX, u32::MAX - 1, u32::MAX - 2]
                                [returns[kind] as usize - 1],
                            _ => unreachable!(),
                        };
                        assert_eq!(m.cpu.r0, expected, "actual original method return r0");
                        println!(
                            "ZFLASH_METHOD_RETURN {}",
                            serde_json::json!({"method":kind,"ordinal":returns[kind],"r0":m.cpu.r0,"return_pc":lr,"elf_sha256":hash})
                        );
                    }
                }
                if let Some(kind) = methods.iter().position(|p| *p == pc) {
                    calls[kind] += 1;
                    active.push((kind, m.cpu.lr & !1));
                }
            }
            assert_eq!(
                m.bus.read_u32(witness + 14 * 4).unwrap(),
                0,
                "actual panic must stay zero"
            );
            if pc == checkpoint {
                let next = m.cpu.r0;
                assert_eq!(next, phase + 1);
                phase = next;
                assert!(
                    active.is_empty(),
                    "actual method LR returns before checkpoint"
                );
                if !batched {
                    let expected = match phase {
                        1 => [2, 1, 0, 0],
                        2 => [2, 1, 1, 0],
                        3 => [2, 1, 1, 1],
                        4 => [2, 1, 1, 2],
                        5 => [2, 1, 1, 5],
                        6 => [2, 1, 2, 5],
                        _ => unreachable!(),
                    };
                    assert_eq!(calls, expected);
                    assert_eq!(returns, calls);
                    let expected_irq = if phase <= 2 {
                        0
                    } else if phase == 3 {
                        2
                    } else {
                        3
                    };
                    assert!(active_irq.is_none());
                    assert_eq!(irq_calls, [expected_irq; 2]);
                    assert_eq!(irq_returns, irq_calls);
                }
                let word = |i: u64| m.bus.read_u32(witness + i * 4).unwrap();
                assert_eq!(word(1), 0x80000);
                assert_eq!(word(2), 8192);
                assert_eq!(m.bus.flash.data[START - 1], 0x5a);
                assert_eq!(m.bus.flash.data[START + 8192], 0xc3);
                assert_eq!(m.bus.read_u16(NVM + 0x12).unwrap() & 1, 1);
                if phase >= 2 {
                    assert_eq!(m.bus.read_u16(NVM + 0x10).unwrap(), 1);
                    assert_eq!(m.bus.read_u16(NVM).unwrap() & 0xc000, 0);
                    assert_eq!(m.bus.read_u32(0x41006008).unwrap(), 1);
                }
                let data = &m.bus.flash.data[START..START + 8192];
                if (phase == 2 || phase == 6) && data.iter().any(|v| *v != 0xff) {
                    assert_eq!(mutant, "omit-erase-commit");
                    assert_eq!(phase, 2);
                    assert_eq!(data[0], 0xa5);
                    assert_eq!(word(3), 0);
                    println!(
                        "ZFLASH_MUTANT_WRONG_ERASE {}",
                        serde_json::json!({"phase":phase,"actual_first":data[0],"calls":calls,"returns":returns,"mutant":mutant,"elf_sha256":hash})
                    );
                    detected = true;
                    break;
                }
                if (3..=5).contains(&phase) {
                    let expected: Vec<u8> = (0..32).map(|i| (i * 7 + 3) as u8).collect();
                    if data[..32] != expected {
                        assert_eq!(mutant, "omit-program-commit");
                        assert_eq!(phase, 3);
                        assert_eq!(data[0], 0xff);
                        assert_eq!(word(4), 0);
                        println!(
                            "ZFLASH_MUTANT_WRONG_PROGRAM {}",
                            serde_json::json!({"phase":phase,"actual_first":data[0],"calls":calls,"returns":returns,"mutant":mutant,"elf_sha256":hash})
                        );
                        detected = true;
                        break;
                    }
                    assert!(data[32..40].iter().all(|v| *v == 0xff));
                    if phase >= 4 {
                        assert_eq!(&data[40..48], &expected[..8]);
                    }
                    assert!(data[48..].iter().all(|v| *v == 0xff));
                }
                if phase == 5 || phase == 6 {
                    assert_eq!(
                        [word(6), word(7), word(8)],
                        [u32::MAX, u32::MAX - 1, u32::MAX - 2]
                    );
                }
                println!(
                    "ZFLASH_PHASE {}",
                    serde_json::json!({"batched":batched,"phase":phase,"calls":calls,"returns":returns,"first":data[0],"ready":true,"neighbors":[0x5a,0xc3],"elf_sha256":hash})
                );
            }
            if m.bus.read_u32(witness).unwrap() == 0x600d {
                break;
            }
            m.step().unwrap();
            fuel += 1;
        }
        if !mutant.is_empty() {
            assert!(detected, "specific wrong memory result required");
            continue;
        }
        assert!(!detected);
        assert_eq!(phase, 6);
        if !batched {
            assert_eq!(calls, [2, 1, 2, 5]);
            assert_eq!(returns, calls);
            assert_eq!(irq_calls, [3, 3]);
            assert_eq!(irq_returns, irq_calls);
        }
        assert_eq!(m.bus.read_u32(witness + 12 * 4).unwrap(), 3);
        assert_eq!(m.bus.read_u32(witness + 13 * 4).unwrap(), 3);
        assert!(!m.cpu.primask && m.cpu.active_exception == 0);
        println!(
            "ACTUAL_ZFLASH_FINITE_PASS {}",
            serde_json::json!({"batched":batched,"phase":phase,"steps":fuel,"calls":calls,"returns":returns,"elf_sha256":hash,"full_board":false,"physical_accuracy":false})
        );
    }
}
