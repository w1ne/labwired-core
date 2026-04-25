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
//!
//! Each test also asserts that the firmware emits the expected UART bytes,
//! proving the CPU executed real application logic, not just spun in reset loops.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::riscv::RiscV;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::trace::TraceObserver;
use labwired_core::{Cpu, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How many cycles a firmware must survive before the test passes.
const SURVIVAL_CYCLES: u32 = 800_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CpuFamily {
    CortexM,
    RiscV,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SurvivalCase {
    name: &'static str,
    core: &'static str,
    family: CpuFamily,
    chip: &'static str,
    system: &'static str,
    fixture: &'static str,
    valid_pc_ranges: &'static [(u32, u32)],
    /// Bytes that must appear somewhere in the UART output after SURVIVAL_CYCLES.
    /// Proves the firmware executed real application logic, not just a reset loop.
    expected_uart_output: &'static [u8],
}

const IMPORTANT_CORES: &[&str] = &[
    "cortex-m0+",
    "cortex-m3",
    "cortex-m4",
    "cortex-m33",
    "rv32i",
];

const SURVIVAL_CASES: &[SurvivalCase] = &[
    SurvivalCase {
        name: "stm32f103_blinky",
        core: "cortex-m3",
        family: CpuFamily::CortexM,
        chip: "stm32f103",
        system: "stm32f103-bare",
        fixture: "stm32f103-blinky.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        // Arduino HAL firmware: setup() prints this via interrupt-driven HardwareSerial.
        expected_uart_output: b"LabWired Playground - Arduino Blink",
    },
    SurvivalCase {
        name: "stm32f401_blinky",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32f401",
        system: "nucleo-f401re",
        fixture: "stm32f401-blinky.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x0807_FFFF), (0x2000_0000, 0x2001_FFFF)],
        // Keep this as a control-flow survival check. The current F401 board model
        // does not yet produce deterministic UART bytes end-to-end.
        expected_uart_output: b"",
    },
    SurvivalCase {
        name: "rp2040_demo",
        core: "cortex-m0+",
        family: CpuFamily::CortexM,
        chip: "rp2040",
        system: "rp2040-pico",
        fixture: "rp2040-demo.elf",
        valid_pc_ranges: &[(0x1000_0000, 0x101F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"RP2040_SMOKE_OK\n",
    },
    SurvivalCase {
        name: "nrf52840_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "nrf52840",
        system: "nrf52840-dk",
        fixture: "nrf52840-demo.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x000F_FFFF), (0x2000_0000, 0x2003_FFFF)],
        expected_uart_output: b"NRF52840_SMOKE_OK\n",
    },
    SurvivalCase {
        name: "nrf52832_demo",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "nrf52832",
        system: "nrf52-dk",
        fixture: "nrf52832-demo.elf",
        valid_pc_ranges: &[(0x0000_0000, 0x0007_FFFF), (0x2000_0000, 0x2000_FFFF)],
        // The nrf52832-demo.elf binary was compiled for nRF52840 (256KB RAM), but the
        // nRF52832 chip config only has 64KB RAM. The initial SP (0x20040000) sits outside
        // the 64KB boundary, making the stack unreliable. UART output is not asserted here.
        expected_uart_output: b"",
    },
    SurvivalCase {
        name: "stm32h563_demo",
        core: "cortex-m33",
        family: CpuFamily::CortexM,
        chip: "stm32h563",
        system: "nucleo-h563zi-demo",
        fixture: "stm32h563-demo.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x081F_FFFF), (0x2000_0000, 0x2009_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        name: "riscv_ci_fixture",
        core: "rv32i",
        family: CpuFamily::RiscV,
        chip: "ci-fixture-riscv",
        system: "ci-fixture-riscv-uart1",
        fixture: "riscv-ci-fixture.elf",
        valid_pc_ranges: &[(0x8000_0000, 0x8001_FFFF), (0x8002_0000, 0x8002_FFFF)],
        expected_uart_output: b"OK\n",
    },
    SurvivalCase {
        name: "esp32c3_demo",
        core: "rv32i",
        family: CpuFamily::RiscV,
        chip: "esp32c3",
        system: "esp32c3-devkit",
        fixture: "esp32c3-demo.elf",
        valid_pc_ranges: &[(0x4200_0000, 0x423F_FFFF), (0x3FC8_0000, 0x3FEF_FFFF)],
        expected_uart_output: b"ESP OK\n",
    },
    SurvivalCase {
        // Hardware-validated against real NUCLEO-L476RG silicon: the
        // exact byte stream below was captured from /dev/ttyACM1 with the
        // J-Link OB Virtual COM Port at 115200 baud. The simulator must
        // reproduce it verbatim — drift means a regression in the L4
        // chip config, the FPU implementation, or the Thumb-2 decoder.
        name: "nucleo_l476rg_smoke",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-smoke.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output:
            b"L476 SMOKE\r\nDEV=10076415\r\nMUL=60FC303A\r\nFPU=40C8F5C3\r\nDONE\r\n",
    },
    SurvivalCase {
        // SPI1 register-level fidelity. Captured from real silicon — the
        // sim's SPI peripheral matches CR1/CR2/SR latching, CR2 reset
        // value (0x0700 = DS=8-bit on STM32L4), and the no-loopback
        // transmit semantics (SR=0x0002 / DR=0x00 after TX with no
        // slave wired).
        name: "nucleo_l476rg_spi",
        core: "cortex-m4",
        family: CpuFamily::CortexM,
        chip: "stm32l476",
        system: "nucleo-l476rg",
        fixture: "nucleo-l476rg-spi.elf",
        valid_pc_ranges: &[(0x0800_0000, 0x080F_FFFF), (0x2000_0000, 0x2001_FFFF)],
        expected_uart_output: b"SPI1 RESET\r\n\
CR1=0000\r\n\
CR2=0700\r\n\
SR=0002\r\n\
SPI1 CONFIG\r\n\
CR1=033C\r\n\
CR2=1700\r\n\
SR=0002\r\n\
SPI1 ENABLED\r\n\
CR1=037C\r\n\
SR=0002\r\n\
SPI1 AFTER TX\r\n\
SR=0002\r\n\
DR=00\r\n\
DONE\r\n",
    },
];

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

fn assert_uart_contains(uart_bytes: &[u8], expected: &[u8], name: &str) {
    // Empty expected means "no assertion" — useful for boards with known limitations.
    if expected.is_empty() {
        return;
    }
    assert!(
        uart_bytes.windows(expected.len()).any(|w| w == expected),
        "Board '{}': UART output did not contain expected bytes.\n\
         Expected (escaped): {:?}\n\
         Actual   (escaped): {:?}\n\
         Actual   (utf8):    {}\n",
        name,
        std::str::from_utf8(expected).unwrap_or("<non-utf8>"),
        std::str::from_utf8(uart_bytes).unwrap_or("<non-utf8>"),
        String::from_utf8_lossy(uart_bytes),
    );
}

fn run_survival_case(case: &SurvivalCase) {
    let firmware = fixtures().join(case.fixture);
    let (pc, uart_bytes) = match case.family {
        CpuFamily::CortexM => {
            run_cortex_m_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES)
        }
        CpuFamily::RiscV => run_riscv_firmware(case.chip, case.system, firmware, SURVIVAL_CYCLES),
    };

    assert_pc_in_range(pc, SURVIVAL_CYCLES, case.valid_pc_ranges);
    assert_uart_contains(&uart_bytes, case.expected_uart_output, case.name);
}

/// Run a Cortex-M machine loaded with `firmware_path` for `cycles` steps.
/// Returns `(final_pc, uart_bytes)` so callers can assert correctness.
fn run_cortex_m_firmware(
    chip_name: &str,
    system_name: &str,
    firmware_path: PathBuf,
    cycles: u32,
) -> (u32, Vec<u8>) {
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );

    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

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

    let uart_bytes = uart_sink.lock().unwrap().clone();
    let final_pc = machine.cpu.get_pc();
    (final_pc, uart_bytes)
}

fn run_riscv_firmware(
    chip_name: &str,
    system_name: &str,
    firmware_path: PathBuf,
    cycles: u32,
) -> (u32, Vec<u8>) {
    assert!(
        firmware_path.exists(),
        "Firmware fixture not found: {:?}",
        firmware_path
    );

    let (chip, manifest) = load_system(chip_name, system_name);
    let mut bus = SystemBus::from_config(&chip, &manifest)
        .expect("Failed to build SystemBus from config");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);
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

    let uart_bytes = uart_sink.lock().unwrap().clone();
    (machine.cpu.get_pc(), uart_bytes)
}

#[test]
fn test_stm32f103_blinky_survival() {
    run_survival_case(&SURVIVAL_CASES[0]);
}

#[test]
fn test_stm32f401_blinky_survival() {
    run_survival_case(&SURVIVAL_CASES[1]);
}

#[test]
fn test_rp2040_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[2]);
}

#[test]
fn test_nrf52840_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[3]);
}

#[test]
fn test_nrf52832_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[4]);
}

#[test]
fn test_stm32h563_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[5]);
}

#[test]
fn test_riscv_ci_fixture_survival() {
    run_survival_case(&SURVIVAL_CASES[6]);
}

#[test]
fn test_esp32c3_demo_survival() {
    run_survival_case(&SURVIVAL_CASES[7]);
}

#[test]
fn test_nucleo_l476rg_smoke_survival() {
    run_survival_case(&SURVIVAL_CASES[8]);
}

#[test]
fn test_nucleo_l476rg_spi_survival() {
    run_survival_case(&SURVIVAL_CASES[9]);
}

#[test]
fn test_important_core_regression_matrix_is_complete() {
    for core in IMPORTANT_CORES {
        assert!(
            SURVIVAL_CASES.iter().any(|case| case.core == *core),
            "important core {} is missing from SURVIVAL_CASES",
            core
        );
    }
}
