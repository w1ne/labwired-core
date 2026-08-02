// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F103 DMA1 executing walk-vs-scheduler fidelity differential (on the
//! REAL `stm32f103` `from_config` bus).
//!
//! Closes the DMA gap: the only prior DMA differential
//! (`stm32_dma_walk_differential`) drives the `Dma1` model on a hand-built,
//! chip-agnostic bus. This one builds the actual F103 chip bus — DMA1 mapped at
//! its silicon base `0x4002_0000`, clock-gated behind `RCC_AHBENR.DMA1EN`
//! (RM0008 §7) exactly as the shipped F1 DMA labs see it — and runs a
//! CPU-executed memory-to-memory transfer twice: once with the `Dma1` model
//! pinned onto the legacy per-cycle walk (reference) and once scheduler-driven.
//!
//! After EVERY instruction, the full architectural state (total_cycles, PC, all
//! 16 core registers), the DMA destination buffer, and the polled DMA ISR word
//! must be byte-identical — pinning the per-element transfer cadence, the
//! TCIF/GIF latch cycle, and the copied bytes on the real chip's DMA wiring.
//!
//! Divergence is a real fidelity bug in the F103 DMA scheduler migration and
//! must be reported, never masked.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::dma::Dma1;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;

const DMA_BASE: u32 = 0x4002_0000;
// RCC_AHBENR (RM0008 §7.3.6); DMA1EN is bit 0. Writes to DMA1 are dropped until
// the clock is enabled, so the harness enables it before the run (identically in
// both lanes).
const RCC_AHBENR: u64 = 0x4002_1014;

// SRAM layout (20 KiB at 0x2000_0000 on the F103C8).
const MAIN_COUNT_ADDR: u64 = 0x2000_0000;
const SRC_ADDR: u32 = 0x2000_0100;
const DST_ADDR: u32 = 0x2000_0200;
const FW_BASE: u64 = 0x2000_1000;
const INITIAL_SP: u32 = 0x2000_5000;
const N: u32 = 16;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_machine(reference: bool) -> Machine<labwired_core::cpu::CortexM> {
    let chip_path = workspace_root().join("configs/chips/stm32f103.yaml");
    let sys_path = workspace_root().join("configs/systems/stm32f103-bare.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load stm32f103 chip");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("load stm32f103-bare system");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build F103 bus");

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    if reference {
        let idx = bus
            .find_peripheral_index_by_name("dma1")
            .expect("F103 bus registers dma1");
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Dma1>()
            .expect("dma1 is the Dma1 model")
            .force_legacy_walk();
        bus.recompute_walk_deletable();
    }

    // Enable the DMA1 clock (RCC_AHBENR.DMA1EN) and fill the source buffer — both
    // are identical setup in the two lanes.
    bus.write_u32(RCC_AHBENR, 0x1).expect("enable DMA1 clock");
    for i in 0..N {
        bus.write_u8(SRC_ADDR as u64 + i as u64, 0xA0 ^ (i as u8).wrapping_mul(7))
            .unwrap();
    }

    // mem2mem poll firmware in SRAM (see module docs). CCR =
    // EN|DIR|PINC|MINC|MEM2MEM (bits 0,4,6,7,14 = 0x40D1); no TCIE (polled).
    let fw: [u16; 16] = [
        0x4807,
        0x4908,
        0x6101,
        0x4908,
        0x6141,
        0x2100 | (N as u16),
        0x60C1,
        0x4907,
        0x6081,
        0x4A07,
        0x2300,
        0x3301,
        0x6013,
        0x6804,
        0xE7FB,
        0xBF00,
    ];
    for (i, hw) in fw.iter().enumerate() {
        bus.write_u16(FW_BASE + (i as u64) * 2, *hw).unwrap();
    }
    // Literal pool at FW_BASE + 0x20.
    bus.write_u32(FW_BASE + 0x20, DMA_BASE).unwrap();
    bus.write_u32(FW_BASE + 0x24, DST_ADDR).unwrap();
    bus.write_u32(FW_BASE + 0x28, SRC_ADDR).unwrap();
    bus.write_u32(FW_BASE + 0x2C, 0x40D1).unwrap();
    bus.write_u32(FW_BASE + 0x30, MAIN_COUNT_ADDR as u32)
        .unwrap();

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine.cpu.sp = INITIAL_SP;
    machine
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    isr: u32,
    dst: Vec<u8>,
}

fn probe(m: &Machine<labwired_core::cpu::CortexM>, step: u64) -> Probe {
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = m.read_core_reg(i as u8);
    }
    let dst = (0..N)
        .map(|i| m.bus.read_u8(DST_ADDR as u64 + i as u64).unwrap())
        .collect();
    Probe {
        step,
        total_cycles: m.total_cycles,
        pc: m.cpu.get_pc(),
        regs,
        isr: m.bus.read_u32(DMA_BASE as u64).unwrap(),
        dst,
    }
}

fn run_probed(reference: bool, steps: u64) -> Vec<Probe> {
    let mut m = build_machine(reference);
    m.cpu.pc = FW_BASE as u32;
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        m.run(Some(1)).unwrap();
        probes.push(probe(&m, s + 1));
    }
    probes
}

/// mem2mem transfer on the real F103 DMA1: walk reference vs scheduler,
/// byte-identical after every instruction, and the destination holds the copied
/// source pattern with TCIF latched.
#[test]
fn stm32f103_dma_mem2mem_walk_vs_scheduler_is_byte_identical() {
    const STEPS: u64 = 800;
    let walk = run_probed(true, STEPS);
    let sched = run_probed(false, STEPS);

    assert_eq!(walk.len(), sched.len());
    for (w, s) in walk.iter().zip(sched.iter()) {
        assert_eq!(
            w, s,
            "F103 DMA mem2mem: first divergence at step {} (walk vs scheduler)",
            w.step
        );
    }

    // The transfer must have actually completed on the real chip bus.
    let last = walk.last().unwrap();
    let expected: Vec<u8> = (0..N).map(|i| 0xA0u8 ^ (i as u8).wrapping_mul(7)).collect();
    assert_eq!(
        last.dst, expected,
        "F103 DMA1 mem2mem must copy the source bytes to the destination"
    );
    // TCIF1 (bit 1) latched in the DMA ISR register at completion.
    assert!(
        walk.iter().any(|p| p.isr & (1 << 1) != 0),
        "F103 DMA1 must latch TCIF1 on transfer completion"
    );
}
