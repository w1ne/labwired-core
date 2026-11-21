// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32 TIM1/TIM8 update and capture/compare events pend their silicon lines.
//!
//! An advanced-control timer has separate NVIC vectors for update (`TIMx_UP`)
//! and capture/compare (`TIMx_CC`). These descriptors once routed both event
//! classes to one line, and on G4, WB and L4 that line was the wrong one:
//! G431/G474/WB55 TIM1 sat on 24 (TIM1_BRK), L476/L432 TIM1 on 27 (TIM1_CC)
//! and L476 TIM8 on 46 (TIM8_CC). Firmware that enabled `TIM1_UP_IRQn` never
//! saw an update interrupt. The F103 ADC2 also had no line; on silicon it
//! shares ADC1_2_IRQn = 18 with ADC1. Numbers are from the vendor CMSIS
//! device headers (`stm32f103xb.h`, `stm32g431xx.h`, `stm32l476xx.h`, ...).
//!
//! TIF, COMIF and BIF had no line at all: the timer model masked SR with
//! UIF|CC1..4IF only. They now pend TIMx_TRG_COM / TIMx_BRK where the
//! descriptor names them (`trigger_com_irq`, `break_irq`), and the timer's
//! own line otherwise.
//!
//! Each case arms one event with the line masked in the NVIC and reads ISPR:
//! the expected line must pend and the other one must not.

use crate::common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};
use std::sync::atomic::Ordering;

const DIER: u64 = 0x0C;
const SR: u64 = 0x10;
const EGR: u64 = 0x14;
const UIE_UG: u32 = 1 << 0;
const CC1IE_CC1G: u32 = 1 << 1;

/// `b .` — lets `Machine::run` tick peripherals without touching MMIO.
const SPIN: u16 = 0xE7FE;

/// (descriptor, timer id, base, update line, capture/compare line)
const TIMERS: &[(&str, &str, u64, u32, u32)] = &[
    ("stm32f103", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103c4", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103c6", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103r4", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103r4h", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103r6", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103r6h", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103r8", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103rb", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103t4", "tim1", 0x4001_2C00, 25, 27),
    ("stm32f103t6", "tim1", 0x4001_2C00, 25, 27),
    ("stm32g431rb", "tim1_pwm", 0x4001_2C00, 25, 27),
    ("stm32g474re", "tim1_pwm", 0x4001_2C00, 25, 27),
    ("stm32wb55", "tim1_pwm", 0x4001_2C00, 25, 27),
    ("stm32l476", "tim1", 0x4001_2C00, 25, 27),
    ("stm32l476", "tim8", 0x4001_3400, 44, 46),
    ("stm32l432kc", "tim1", 0x4001_2C00, 25, 27),
    ("ci-fixture-stm32l476-readonly", "tim1", 0x4001_2C00, 25, 27),
    ("ci-fixture-stm32l476-readonly", "tim8", 0x4001_3400, 44, 46),
    ("stm32l552", "tim1", 0x4001_2C00, 42, 44),
    ("stm32l552", "tim8", 0x4001_3400, 52, 54),
    ("stm32h563", "tim1_pwm", 0x4001_2C00, 42, 44),
    ("stm32u545", "tim1_pwm", 0x4001_2C00, 42, 44),
    ("stm32u575", "tim1_pwm", 0x4001_2C00, 42, 44),
    ("stm32u5a5zj", "tim1_pwm", 0x4001_2C00, 42, 44),
    ("stm32wba52", "tim1_pwm", 0x4001_2C00, 38, 40),
    ("stm32f072", "tim1", 0x4001_2C00, 13, 14),
    ("onboarding/stm32f042", "tim1", 0x4001_2C00, 13, 14),
    ("onboarding/stm32f072", "tim1", 0x4001_2C00, 13, 14),
    ("stm32g071", "tim1_pwm", 0x4001_2C00, 13, 14),
    ("stm32c031c6", "tim1_pwm", 0x4001_2C00, 13, 14),
];

/// Builds the descriptor with every clock gate removed, so the test does not
/// need each family's RCC layout. A spin loop in RAM keeps the CPU busy.
fn machine(stem: &str) -> Machine<CortexM> {
    let chip_path = root(&format!("configs/chips/{stem}.yaml"));
    let mut chip = ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    for p in &mut chip.peripherals {
        p.clock = None;
    }
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "schema_version: \"1.0\"\nname: adv-timer-irq\nchip: \"{}\"\n",
        chip_path.display()
    ))
    .expect("minimal manifest");
    let spin_pc = chip.ram.base as u32 + 0x100;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    bus.write_u16(u64::from(spin_pc), SPIN).expect("spin loop");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.cpu.pc = spin_pc;
    machine.cpu.sp = chip.ram.base as u32 + 0x1000;
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine
}

fn ispr(machine: &Machine<CortexM>, irq: u32) -> bool {
    let nvic = machine.bus.nvic.as_ref().expect("configured NVIC");
    (nvic.ispr[(irq / 32) as usize].load(Ordering::SeqCst) & (1 << (irq % 32))) != 0
}

fn clear_pend(machine: &Machine<CortexM>, irq: u32) {
    let nvic = machine.bus.nvic.as_ref().expect("configured NVIC");
    nvic.ispr[(irq / 32) as usize].fetch_and(!(1 << (irq % 32)), Ordering::SeqCst);
}

/// Arms `bits` in DIER and EGR, runs a few cycles and returns the pend state
/// of (`want`, `other`). The timer is left with DIER/SR clear.
fn fire(
    machine: &mut Machine<CortexM>,
    base: u64,
    bits: u32,
    want: u32,
    other: u32,
) -> (bool, bool) {
    machine.bus.write_u32(base + DIER, bits).expect("DIER");
    machine.bus.write_u32(base + EGR, bits).expect("EGR");
    for _ in 0..8 {
        machine.run(Some(1)).expect("spin loop");
        if ispr(machine, want) {
            break;
        }
    }
    let seen = (ispr(machine, want), ispr(machine, other));
    machine.bus.write_u32(base + DIER, 0).expect("DIER clear");
    machine.bus.write_u32(base + SR, 0).expect("SR clear");
    clear_pend(machine, want);
    clear_pend(machine, other);
    seen
}

#[test]
fn advanced_timers_pend_update_and_cc_on_their_own_lines() {
    let mut failures = Vec::new();
    for &(stem, id, base, up, cc) in TIMERS {
        let chip = ChipDescriptor::from_file(root(&format!("configs/chips/{stem}.yaml")))
            .expect("chip yaml");
        let p = chip
            .peripherals
            .iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{stem}: no {id}"));
        assert_eq!(p.base_address, base, "{stem}: {id} base");

        let mut m = machine(stem);
        let update = fire(&mut m, base, UIE_UG, up, cc);
        let compare = fire(&mut m, base, CC1IE_CC1G, cc, up);
        if update != (true, false) {
            failures.push(format!(
                "{stem} {id}: UG pended line {up}={} line {cc}={}",
                update.0, update.1
            ));
        }
        if compare != (true, false) {
            failures.push(format!(
                "{stem} {id}: CC1G pended line {cc}={} line {up}={}",
                compare.0, compare.1
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Break (BIF), commutation (COMIF) and trigger (TIF) events pend
/// TIMx_BRK = UP - 1 and TIMx_TRG_COM = UP + 1 where the chip splits them
/// (CC = UP + 2), and the single TIMx_BRK_UP_TRG_COM line on F0/G0/C0.
#[test]
fn advanced_timers_pend_break_and_trigger_com_on_their_lines() {
    const COMIE_COMG: u32 = 1 << 5;
    const TIE_TG: u32 = 1 << 6;
    const BIE_BG: u32 = 1 << 7;
    let mut failures = Vec::new();
    for &(stem, id, base, up, cc) in TIMERS {
        let split = cc == up + 2;
        let (brk, trg_com, other) = if split {
            (up - 1, up + 1, up)
        } else {
            (up, up, cc)
        };
        let mut m = machine(stem);
        for (name, bits, want) in [
            ("BG", BIE_BG, brk),
            ("COMG", COMIE_COMG, trg_com),
            ("TG", TIE_TG, trg_com),
        ] {
            let seen = fire(&mut m, base, bits, want, other);
            if seen != (true, false) {
                failures.push(format!(
                    "{stem} {id}: {name} pended line {want}={} line {other}={}",
                    seen.0, seen.1
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// F103 ADC2 end-of-conversion pends ADC1_2_IRQn = 18, the line it shares
/// with ADC1 (RM0008 table 63).
#[test]
fn f103_adc2_eoc_pends_shared_adc1_2_line() {
    const ADC2: u64 = 0x4001_2800;
    const ADC_SR: u64 = ADC2;
    const ADC_CR1: u64 = ADC2 + 0x04;
    const ADC_CR2: u64 = ADC2 + 0x08;
    const EOCIE: u32 = 1 << 5;
    const ADON: u32 = 1 << 0;
    // EXTSEL = 111 (SWSTART), EXTTRIG, SWSTART (RM0008 §11.12.3).
    const SW_START: u32 = (0x7 << 17) | (1 << 20) | (1 << 22);
    const ADC1_2: u32 = 18;

    for stem in [
        "stm32f103",
        "stm32f103c4",
        "stm32f103c6",
        "stm32f103r4",
        "stm32f103r4h",
        "stm32f103r6",
        "stm32f103r6h",
        "stm32f103r8",
        "stm32f103rb",
        "stm32f103t4",
        "stm32f103t6",
    ] {
        let mut m = machine(stem);
        m.bus.write_u32(ADC_CR1, EOCIE).expect("CR1");
        m.bus.write_u32(ADC_CR2, ADON).expect("ADON");
        m.bus.write_u32(ADC_CR2, ADON | SW_START).expect("SWSTART");
        let mut pended = false;
        for _ in 0..4096 {
            m.run(Some(1)).expect("spin loop");
            if ispr(&m, ADC1_2) {
                pended = true;
                break;
            }
        }
        let eoc = m.bus.read_u32(ADC_SR).expect("SR") & (1 << 1) != 0;
        assert!(eoc, "{stem}: ADC2 EOC never set");
        assert!(pended, "{stem}: ADC2 EOC with EOCIE did not pend line 18");
    }
}

/// L476 TIM1 update and TIM16 share TIM1_UP_TIM16_IRQn = 25. A store to the
/// idle TIM16 reconciles its low line; it must not drop the pend TIM1 holds.
/// Once TIM1 clears its flag, no source holds the line and the pend drops.
#[test]
fn a_low_peer_keeps_a_shared_line_pended() {
    const TIM1: u64 = 0x4001_2C00;
    const TIM16: u64 = 0x4001_4400;
    const LINE: u32 = 25;

    let mut m = machine("stm32l476");
    m.bus.write_u32(TIM1 + DIER, UIE_UG).expect("DIER");
    m.bus.write_u32(TIM1 + EGR, UIE_UG).expect("EGR");
    for _ in 0..8 {
        m.run(Some(1)).expect("spin loop");
    }
    assert!(ispr(&m, LINE), "TIM1 UG pends line 25");

    m.bus.write_u32(TIM16 + SR, 0).expect("TIM16 SR");
    m.run(Some(1)).expect("spin loop");
    assert!(ispr(&m, LINE), "idle TIM16 dropped TIM1's pend on 25");

    m.bus.write_u32(TIM1 + SR, 0).expect("TIM1 SR");
    assert!(!ispr(&m, LINE), "no source holds 25, the level pend drops");
}
