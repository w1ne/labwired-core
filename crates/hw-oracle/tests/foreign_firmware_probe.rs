// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// Foreign-firmware bring-up triage harness (PC histogram + register dump).
// Not for CI; run explicitly:
//   FOREIGN_ELF=<path> [FOREIGN_STEPS=N] cargo test -p labwired-hw-oracle \
//     --test foreign_firmware_probe -- --ignored --nocapture

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{Cpu, Machine};
use labwired_loader::load_elf;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
#[ignore = "manual probe harness — needs FOREIGN_ELF env"]
fn probe_foreign_firmware() {
    let elf = std::env::var("FOREIGN_ELF").expect("set FOREIGN_ELF");
    let steps: u64 = std::env::var("FOREIGN_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500_000);

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip_path = root.join("configs/chips/stm32h563.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "foreign-probe".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = load_elf(PathBuf::from(&elf).as_path()).expect("load firmware ELF");
    machine.load_firmware(&image).expect("map firmware");

    let watch: Option<u64> = std::env::var("FOREIGN_WATCH")
        .ok()
        .and_then(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok());
    let mut pc_hist: HashMap<u32, u64> = HashMap::new();
    let mut last_csr: Option<(u32, u32, u32)> = None;
    let mut executed = 0u64;
    let mut stop = None;
    for _ in 0..steps {
        let pc = machine.cpu.get_pc();
        *pc_hist.entry(pc).or_insert(0) += 1;
        if std::env::var("FOREIGN_CSRTRACE").is_ok() {
            let csr = machine.bus.read_u32(0x4002_03E0).unwrap_or(0);
            let cbr1 = machine.bus.read_u32(0x4002_0418).unwrap_or(0);
            let cllr = machine.bus.read_u32(0x4002_044C).unwrap_or(0);
            let key = (csr, cbr1, cllr);
            if Some(key) != last_csr {
                eprintln!("step {executed}: CSR={csr:#010x} CBR1={cbr1:#06x} CLLR={cllr:#010x} pc={pc:#010x}");
                last_csr = Some(key);
            }
        }
        if let Some(w) = watch {
            if machine.bus.read_u32(w).unwrap_or(0) != 0 {
                use labwired_core::Cpu;
                println!(
                    "WATCH {w:#x} nonzero at step {executed}, pc {pc:#010x}, lr {:#010x}, active_exc {}",
                    machine.cpu.get_register(14),
                    machine.cpu.active_exception
                );
                break;
            }
        }
        match machine.step() {
            Ok(_) => executed += 1,
            Err(e) => {
                stop = Some(format!("{e:?} @ pc={pc:#010x} after {executed} steps"));
                break;
            }
        }
    }

    let mut hot: Vec<(u32, u64)> = pc_hist.into_iter().collect();
    hot.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    println!(
        "executed {executed} steps; final pc {:#010x}",
        machine.cpu.get_pc()
    );
    if let Some(s) = stop {
        println!("STOPPED: {s}");
    }
    println!("hottest PCs:");
    for (pc, n) in hot.iter().take(12) {
        println!("  {pc:#010x}  x{n}");
    }
    if let Ok(out) = std::env::var("FOREIGN_PCDUMP") {
        let mut lines: Vec<String> = hot
            .iter()
            .map(|(pc, n)| format!("{pc:#010x} {n}"))
            .collect();
        lines.sort();
        std::fs::write(out, lines.join("\n")).unwrap();
    }

    println!(
        "cpu: primask={} active_exception={} pending={:?}",
        machine.cpu.primask, machine.cpu.active_exception, machine.cpu.pending_exceptions
    );
    for (label, addr) in [
        ("TIM12 CR1", 0x4000_1800u64),
        ("TIM12 DIER", 0x4000_180C),
        ("TIM12 SR", 0x4000_1810),
        ("TIM12 CNT", 0x4000_1824),
        ("TIM12 PSC", 0x4000_1828),
        ("TIM12 ARR", 0x4000_182C),
        ("TIM12 CCR1", 0x4000_1834),
        ("TIM1 CR1", 0x4001_2C00),
        ("TIM1 DIER", 0x4001_2C0C),
        ("TIM1 SR", 0x4001_2C10),
        ("TIM1 CNT", 0x4001_2C24),
        ("TIM1 PSC", 0x4001_2C28),
        ("TIM1 ARR", 0x4001_2C2C),
        ("TIM1 CCR1", 0x4001_2C34),
        ("TIM2 CR1", 0x4000_0000),
        ("TIM2 DIER", 0x4000_000C),
        ("TIM2 SR", 0x4000_0010),
        ("TIM2 CNT", 0x4000_0024),
        ("TIM2 PSC", 0x4000_0028),
        ("TIM2 ARR", 0x4000_002C),
        ("TIM2 CCR1", 0x4000_0034),
        ("USART3 CR1", 0x4000_4800),
        ("USART3 ISR", 0x4000_481C),
        ("NVIC ISER1b", 0xE000_E104),
        ("SCB ICSR", 0xE000_ED04),
        ("VEC50", 0x0800_00C8),
        ("VEC49", 0x0800_00C4),
        ("GPDMA C7SR", 0x4002_03E0),
        ("XFER_CPLT", 0x2000_00EC),
        ("CB_CPLT", 0x2000_0150),
        ("CB_HALF", 0x2000_0154),
        ("CB_ERR", 0x2000_0158),
        ("XFER_ERR", 0x2000_00E8),
        ("C7LLR", 0x4002_044C),
        ("C7LBAR", 0x4002_03D0),
        ("C7TR2", 0x4002_0414),
        ("C7SAR", 0x4002_041C),
        ("GPDMA C7CR", 0x4002_03E4),
        ("GPDMA C7BR1", 0x4002_0418),
        ("NVIC ISER3", 0xE000_E10C),
        ("NVIC ISPR3", 0xE000_E20C),
        ("NVIC ISER1", 0xE000_E104),
        ("NVIC ISPR1", 0xE000_E204),
        ("GPIOB ODR", 0x4202_0414),
        ("GPIOG ODR", 0x4202_1814),
        ("RCC CR", 0x4402_0C00),
        ("RCC CFGR1", 0x4402_0C1C),
    ] {
        println!(
            "  {label:<11} = {:#010x}",
            machine.bus.read_u32(addr).unwrap_or(0xDEAD_BEEF)
        );
    }
}
