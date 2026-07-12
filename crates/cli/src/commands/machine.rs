// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired machine` snapshot load/inspect.

use crate::artifacts::Snapshot;
use crate::*;

pub(crate) fn run_machine_load(args: LoadArgs) -> ExitCode {
    info!("Loading machine from snapshot: {:?}", args.snapshot);

    let f = match std::fs::File::open(&args.snapshot) {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to open snapshot {:?}: {}", args.snapshot, e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let snapshot_data: Snapshot = match serde_json::from_reader(f) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to parse snapshot {:?}: {}", args.snapshot, e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let (cpu_snapshot, peripherals_snapshot, config) = match snapshot_data {
        Snapshot::Interactive {
            cpu,
            peripherals,
            config,
            ..
        } => (cpu, peripherals, config),
        _ => {
            error!("Unsupported snapshot type for loading");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Reconstruct bus
    let mut bus = match labwired_core::system::builder::build_system_bus(config.system.as_deref()) {
        Ok(bus) => bus,
        Err(e) => {
            error!("Failed to reconstruct bus: {:#}", e);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Load original firmware (required for memory content that isn't in snapshot yet)
    // Note: Our snapshot currently doesn't include full RAM/Flash dumps to keep it small.
    // So we MUST load the firmware first.
    let program = match labwired_loader::load_elf(&config.firmware) {
        Ok(p) => p,
        Err(e) => {
            error!(
                "Failed to load original firmware {:?}: {:#}",
                config.firmware, e
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let arch = match program.arch {
        labwired_core::Arch::Arm => labwired_config::Arch::Arm,
        labwired_core::Arch::RiscV => labwired_config::Arch::RiscV,
        labwired_core::Arch::XtensaLx7 => labwired_config::Arch::Xtensa,
        _ => {
            error!("Unknown architecture in firmware");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let metrics = Arc::new(labwired_core::metrics::PerformanceMetrics::new());

    match arch {
        labwired_config::Arch::Arm => {
            let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                error!("Failed to load firmware: {}", e);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }

            // Apply snapshot
            machine.cpu.apply_snapshot(&cpu_snapshot);
            for ps in peripherals_snapshot {
                if let Some(state) = ps.state {
                    // Find peripheral by name and restore
                    for p in &mut machine.bus.peripherals {
                        if p.name == ps.name {
                            if let Err(e) = p.dev.restore(state.clone()) {
                                error!("Failed to restore peripheral {}: {}", p.name, e);
                            }
                            break;
                        }
                    }
                }
            }

            info!("Resuming simulation (ARM)...");
            let cli = Cli {
                firmware: Some(config.firmware),
                system: config.system,
                snapshot: None,
                breakpoint: vec![],
                trace: args.trace,
                max_steps: args.max_steps.unwrap_or(config.max_steps),
                gdb: None,
                command: None,
                json: false,
                vcd: None,
            };
            run_simulation_loop(&cli, &mut machine, &metrics);
            ExitCode::from(EXIT_PASS)
        }
        labwired_config::Arch::RiscV => {
            let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                error!("Failed to load firmware: {}", e);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }

            // Apply snapshot
            machine.cpu.apply_snapshot(&cpu_snapshot);
            for ps in peripherals_snapshot {
                if let Some(state) = ps.state {
                    for p in &mut machine.bus.peripherals {
                        if p.name == ps.name {
                            if let Err(e) = p.dev.restore(state.clone()) {
                                error!("Failed to restore peripheral {}: {}", p.name, e);
                            }
                            break;
                        }
                    }
                }
            }

            info!("Resuming simulation (RISC-V)...");
            let cli = Cli {
                firmware: Some(config.firmware),
                system: config.system,
                snapshot: None,
                breakpoint: vec![],
                trace: args.trace,
                max_steps: args.max_steps.unwrap_or(config.max_steps),
                gdb: None,
                command: None,
                json: false,
                vcd: None,
            };
            run_simulation_loop(&cli, &mut machine, &metrics);
            ExitCode::from(EXIT_PASS)
        }
        labwired_config::Arch::Xtensa => {
            let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
            let mut machine = labwired_core::Machine::new(cpu, bus);
            machine.observers.push(metrics.clone());
            if let Err(e) = machine.load_firmware(&program) {
                error!("Failed to load firmware: {}", e);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }

            machine.cpu.apply_snapshot(&cpu_snapshot);
            for ps in peripherals_snapshot {
                if let Some(state) = ps.state {
                    for p in &mut machine.bus.peripherals {
                        if p.name == ps.name {
                            if let Err(e) = p.dev.restore(state.clone()) {
                                error!("Failed to restore peripheral {}: {}", p.name, e);
                            }
                            break;
                        }
                    }
                }
            }

            info!("Resuming simulation (Xtensa)...");
            let cli = Cli {
                firmware: Some(config.firmware),
                system: config.system,
                snapshot: None,
                breakpoint: vec![],
                trace: args.trace,
                max_steps: args.max_steps.unwrap_or(config.max_steps),
                gdb: None,
                command: None,
                json: false,
                vcd: None,
            };
            run_simulation_loop(&cli, &mut machine, &metrics);
            ExitCode::from(EXIT_PASS)
        }
        _ => {
            error!("Unsupported architecture for snapshot load");
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}
