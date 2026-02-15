// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko

use labwired_config::{Arch, ChipDescriptor};
use labwired_core::system;
use labwired_core::Cpu;
use labwired_core::Machine;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_register_compliance_all_chips() -> anyhow::Result<()> {
    // Locate the `core/configs/chips` directory
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Go up two levels from crates/core to reach core/
    let chips_dir = manifest_dir.parent().unwrap().parent().unwrap().join("configs/chips");

    println!("Scanning for chips in: {:?}", chips_dir);

    for entry in fs::read_dir(chips_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            println!("Testing chip: {:?}", path);
            validate_chip(&path)?;
        }
    }

    Ok(())
}

fn validate_chip(path: &PathBuf) -> anyhow::Result<()> {
    let chip = ChipDescriptor::from_file(path)?;

    // Configure Memory (Flash/RAM)
    // Note: SystemBus::new() creates default memories. We resize them.
    // Actually, SystemBus::from_config uses chip.flash/ram directly, so we don't need to manually resize here.
    // We just need to ensure the values are valid.
    let _flash_size = labwired_config::parse_size(&chip.flash.size)? as usize;
    let _ram_size = labwired_config::parse_size(&chip.ram.size)? as usize;
    
    // Create Manual System Manifest
    let dummy_manifest = labwired_config::SystemManifest {
        schema_version: "1.0".to_string(),
        name: "test-bench".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        memory_overrides: Default::default(),
    };
    
    // Using from_config is the best way to get peripherals instantiated correctly
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &dummy_manifest)?;
    
    // Override memory sizes if needed (from_config does it based on chip!)
    // So we don't need manual resize.

    match chip.arch {
        Arch::Arm => {
            let (cpu, _nvic) = system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = Machine::new(cpu, bus);
            validate_registers(&mut machine, &chip)?;
        }
        Arch::RiscV => {
            let cpu = system::riscv::configure_riscv(&mut bus);
            let mut machine = Machine::new(cpu, bus);
            validate_registers(&mut machine, &chip)?;
        }
        Arch::Unknown => {
            println!("Skipping unknown architecture for {:?}", path);
        }
    }

    Ok(())
}

fn validate_registers<C: Cpu>(machine: &mut Machine<C>, chip: &ChipDescriptor) -> anyhow::Result<()> {
    for p in &chip.peripherals {
        println!("  Validating peripheral: {} @ 0x{:x}", p.id, p.base_address);
        
        // Smoke Test: Try to read the base address.
        // Even if it's WriteOnly, the bus/peripheral usually treats read as 0 or 0xDEADBEEF, 
        // but crucially NOT MemoryViolation (which means unmapped).
        
        let addr = p.base_address;
        
        // We use machine.bus directly.
        // We expect Ok(_) or maybe Err(SimulationError::DecodeError) but NOT MemoryViolation.
        // Actually labwired-core usually returns Err(SimulationError::MemoryViolation) if unmapped.
        // Peripheral::read usually returns Ok(val).
        
        match machine.bus.read_u32(addr) {
            Ok(_val) => {
                // Success
            }
            Err(labwired_core::SimulationError::MemoryViolation(a)) => {
                 println!("    ERROR: MemoryViolation at 0x{:x} for peripheral {}", a, p.id);
                 return Err(anyhow::anyhow!("Peripheral {} failed smoke test: MemoryViolation", p.id));
            }
            Err(e) => {
                // Other errors like DecodeError are possible if peripheral handles it explicitly,
                // but usually that means "present but error", which is technically "compliant" in terms of mapping.
                // But generally we expect Ok for offset 0.
                 println!("    WARNING: Error reading 0x{:x} for {}: {:?}", addr, p.id, e);
            }
        }
    }
    Ok(())
}
