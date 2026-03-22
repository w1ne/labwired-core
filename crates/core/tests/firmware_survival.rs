// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware survival tests: load real compiled binaries and assert the simulator
//! runs without crashing for a meaningful number of cycles.
//!
//! These are the ground truth for CPU correctness — if a real firmware can't
//! survive N cycles, something is broken in the instruction decoder or executor.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::riscv::RiscV;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::trace::TraceObserver;
use labwired_core::{Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How many cycles a firmware must survive before the test passes.
const SURVIVAL_CYCLES: u32 = 100_000;

fn workspace_root() -> PathBuf {
    // crates/core → crates → workspace root (core/)
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn fixtures() -> PathBuf {
    workspace_root().join("tests/fixtures")
}

fn chip_config(name: &str) -> PathBuf {
    workspace_root().join("configs/chips").join(format!("{name}.yaml"))
}

fn system_config(name: &str) -> PathBuf {
    workspace_root().join("configs/systems").join(format!("{name}.yaml"))
}

fn load_system(chip_name: &str, system_name: &str) -> (ChipDescriptor, SystemManifest) {
    let chip = ChipDescriptor::from_file(&chip_config(chip_name))
        .unwrap_or_else(|e| panic!("Failed to load chip {chip_name}: {e}"));

    let sys_path = system_config(system_name);
    let mut manifest = SystemManifest::from_file(&sys_path)
        .unwrap_or_else(|e| panic!("Failed to load system {system_name}: {e}"));

    manifest.chip = sys_path.parent().unwrap().join(&manifest.chip)
        .to_str().unwrap().to_string();

    (chip, manifest)
}

fn assert_pc_in_range(pc: u32, cycles: u32, ranges: &[(u32, u32)]) {
    assert!(
        ranges.iter().any(|(start, end)| (*start..=*end).contains(&pc)),
        "PC={:#010x} after {} cycles — jumped to unmapped region",
        pc,
        cycles
    );
}

/// Run a Cortex-M machine loaded with `firmware_path` for `cycles` steps.
/// Returns the final PC so callers can assert it landed in flash.
fn run_cortex_m_firmware(
    chip_name: &str,
    system_name: &str,
    firmware_path: PathBuf,
    cycles: u32,
) -> u32 {
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );

    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink, false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.observers.push(trace.clone());

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("Failed to load ELF {:?}: {e}", firmware_path));
    machine.load_firmware(&image)
        .expect("Failed to load firmware into machine");

    let mut last_state: std::collections::VecDeque<(u32, u32, u32)> =
        std::collections::VecDeque::new();
    for step in 0..cycles {
        let pc_before = machine.cpu.get_pc();
        let lr_before = machine.cpu.lr;
        last_state.push_back((step, pc_before, lr_before));
        if last_state.len() > 30 {
            last_state.pop_front();
        }

        machine.step().unwrap_or_else(|e| {
            eprintln!("Last 30 steps before crash:");
            for (s, p, lr) in &last_state {
                eprintln!("  step {:5}: PC={:#010x}  LR={:#010x}", s, p, lr);
            }
            eprintln!("Last instruction traces before crash:");
            for t in trace.take_traces().into_iter().rev().take(24).rev() {
                let lr = t.register_delta.get(&14).map(|(_, new)| *new);
                let sp = t.register_delta.get(&13).map(|(_, new)| *new);
                let pc = t.register_delta.get(&15).map(|(_, new)| *new);
                eprintln!(
                    "  trace pc={:#010x} opcode={:#010x} lr={} sp={} next_pc={}",
                    t.pc,
                    t.instruction,
                    lr.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                    sp.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                    pc.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                );
            }
            panic!(
                "Simulation crashed at step {} (PC={:#010x}): {}",
                step, pc_before, e
            )
        });
    }

    machine.cpu.get_pc()
}

fn run_riscv_firmware(
    chip_name: &str,
    system_name: &str,
    firmware_path: PathBuf,
    cycles: u32,
) -> u32 {
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );

    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink, false);
    let mut machine = Machine::new(RiscV::new(), bus);
    let trace = Arc::new(TraceObserver::new(5000));
    machine.observers.push(trace.clone());

    let image = labwired_loader::load_elf(&firmware_path)
        .unwrap_or_else(|e| panic!("Failed to load ELF {:?}: {e}", firmware_path));
    machine.load_firmware(&image)
        .expect("Failed to load firmware into machine");

    let mut last_pcs: std::collections::VecDeque<(u32, u32)> = std::collections::VecDeque::new();
    for step in 0..cycles {
        let pc_before = machine.cpu.get_pc();
        last_pcs.push_back((step, pc_before));
        if last_pcs.len() > 30 {
            last_pcs.pop_front();
        }

        machine.step().unwrap_or_else(|e| {
            eprintln!("Last 30 steps before crash:");
            for (s, p) in &last_pcs {
                eprintln!("  step {:5}: PC={:#010x}", s, p);
            }
            eprintln!("Last instruction traces before crash:");
            for t in trace.take_traces().into_iter().rev().take(24).rev() {
                let pc = t.register_delta.get(&32).map(|(_, new)| *new);
                eprintln!(
                    "  trace pc={:#010x} opcode={:#010x} next_pc={}",
                    t.pc,
                    t.instruction,
                    pc.map(|v| format!("{v:#010x}")).unwrap_or_else(|| "-".to_string()),
                );
            }
            panic!(
                "Simulation crashed at step {} (PC={:#010x}): {}",
                step, pc_before, e
            )
        });
    }

    machine.cpu.get_pc()
}

// ─── STM32F103 (Cortex-M3) ────────────────────────────────────────────────

#[test]
fn test_stm32f103_blinky_survival() {
    let firmware = fixtures().join("stm32f103-blinky.elf");
    let pc = run_cortex_m_firmware("stm32f103", "stm32f103-bare", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
    );
}

// ─── STM32F401 (Cortex-M4) ────────────────────────────────────────────────

#[test]
fn test_stm32f401_blinky_survival() {
    let firmware = fixtures().join("stm32f401-blinky.elf");
    let pc = run_cortex_m_firmware("stm32f401", "nucleo-f401re", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
    );
}

#[test]
fn test_rp2040_demo_survival() {
    let firmware = fixtures().join("rp2040-demo.elf");
    let pc = run_cortex_m_firmware("rp2040", "rp2040-pico", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x1000_0000, 0x101F_FFFF), (0x2000_0000, 0x2003_FFFF)],
    );
}

#[test]
fn test_nrf52840_demo_survival() {
    let firmware = fixtures().join("nrf52840-demo.elf");
    let pc = run_cortex_m_firmware("nrf52840", "nrf52840-dk", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2003_FFFF)],
    );
}

#[test]
fn test_nrf52832_demo_survival() {
    let firmware = fixtures().join("nrf52832-demo.elf");
    let pc = run_cortex_m_firmware("nrf52832", "nrf52-dk", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2000_FFFF)],
    );
}

#[test]
fn test_stm32h563_demo_survival() {
    let firmware = fixtures().join("stm32h563-demo.elf");
    let pc = run_cortex_m_firmware("stm32h563", "nucleo-h563zi-demo", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x2009_FFFF)],
    );
}

#[test]
fn test_riscv_ci_fixture_survival() {
    let firmware = fixtures().join("riscv-ci-fixture.elf");
    let pc = run_riscv_firmware(
        "ci-fixture-riscv",
        "ci-fixture-riscv-uart1",
        firmware,
        SURVIVAL_CYCLES,
    );

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x8000_0000, 0x8001_FFFF), (0x8002_0000, 0x8002_FFFF)],
    );
}

#[test]
fn test_esp32c3_demo_survival() {
    let firmware = fixtures().join("esp32c3-demo.elf");
    let pc = run_riscv_firmware("esp32c3", "esp32c3-devkit", firmware, SURVIVAL_CYCLES);

    assert_pc_in_range(
        pc,
        SURVIVAL_CYCLES,
        &[(0x4200_0000, 0x423F_FFFF), (0x3FC8_0000, 0x3FCE_3FFF)],
    );
}
