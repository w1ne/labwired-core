// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F103 RTC through the shipped chip config: the counter holds its reset
//! value until firmware sets it up, RSF sets again after the HAL clears it,
//! and the counter advances one second per 72 M cycles from the 32.768 kHz
//! LSE with PRL = 32767 (RM0008 §7.3.9, §18).
//!
//! The stock STM32duino sketch behind this (STM32RTC 1.8.0: setTime, then
//! getSeconds around delay(2100)) reads 56 -> 58 on this model. On the old
//! register bank `HAL_RTC_WaitForSynchro` timed out and the time stayed 56.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus, Machine,
};

const RCC_APB1ENR: u64 = 0x4002_101C;
const RCC_BDCR: u64 = 0x4002_1020;
const PWR_CR: u64 = 0x4000_7000;
const RTC_CRL: u64 = 0x4000_2804;
const RTC_PRLL: u64 = 0x4000_280C;
const RTC_CNTH: u64 = 0x4000_2818;
const RTC_CNTL: u64 = 0x4000_281C;
const BKPEN: u32 = 1 << 27;
const PWREN: u32 = 1 << 28;
const DBP: u32 = 1 << 8;
const LSEON: u32 = 1 << 0;
const RTCSEL_LSE: u32 = 1 << 8;
const RTCEN: u32 = 1 << 15;
const RSF: u32 = 1 << 3;
const CNF: u32 = 1 << 4;
const RTOFF: u32 = 1 << 5;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A machine parked on a `b .` loop in flash, so the host drives MMIO.
fn idle_machine() -> Machine<labwired_core::cpu::cortex_m::CortexM> {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32f103.yaml")).unwrap();
    let manifest =
        SystemManifest::from_file(root().join("validation/arduino-matrix/systems/stm32f103.yaml"))
            .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let mut bytes = vec![0u8; 0x104];
    bytes[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes()); // Thumb entry at 0x100
    bytes[0x100..0x102].copy_from_slice(&0xe7feu16.to_le_bytes()); // b .
    let mut image = ProgramImage::new(0x0800_0101, Arch::Arm);
    image.add_segment(0x0800_0000, bytes);
    machine.load_firmware(&image).unwrap();
    machine
}

fn cnt(bus: &mut SystemBus) -> u32 {
    (bus.read_u32(RTC_CNTH).unwrap() << 16) | bus.read_u32(RTC_CNTL).unwrap()
}

/// The STM32duino/HAL bring-up: backup-domain access, LSE as RTCCLK, RTCEN.
fn enable_rtc_clock(bus: &mut SystemBus) {
    bus.write_u32(RCC_APB1ENR, PWREN | BKPEN).unwrap();
    bus.write_u32(PWR_CR, DBP).unwrap();
    bus.write_u32(RCC_BDCR, LSEON | RTCSEL_LSE | RTCEN).unwrap();
}

#[test]
fn rtc_reads_its_reset_values_until_firmware_writes_it() {
    let mut machine = idle_machine();
    let bus = &mut machine.bus;
    bus.set_current_cycle(10 * 72_000_000);
    assert_eq!(bus.read_u32(RTC_CRL).unwrap(), RTOFF, "CRL reset value");
    assert_eq!(cnt(bus), 0, "the counter does not run before it is set up");
}

/// `HAL_RTC_WaitForSynchro` clears RSF and polls until it is set again.
#[test]
fn rsf_resynchronises_after_the_hal_clears_it() {
    let mut machine = idle_machine();
    let bus = &mut machine.bus;
    enable_rtc_clock(bus);
    bus.write_u32(RTC_CRL, 0).unwrap();
    let mut polls = 0;
    while bus.read_u32(RTC_CRL).unwrap() & RSF == 0 {
        polls += 1;
        assert!(polls < 1_000_000, "RSF never set again");
        bus.set_current_cycle(bus.current_cycle + 64);
    }
}

#[cfg(feature = "event-scheduler")]
#[test]
fn counter_advances_one_second_per_72m_cycles() {
    let mut machine = idle_machine();
    let bus = &mut machine.bus;
    enable_rtc_clock(bus);
    // HAL_RTC_Init + HAL_RTC_SetTime: CNF, PRL = 32767, CNT = 12:34:56.
    bus.write_u32(RTC_CRL, CNF).unwrap();
    bus.write_u32(RTC_PRLL, 0x7FFF).unwrap();
    bus.write_u32(RTC_CNTH, 45_296 >> 16).unwrap();
    bus.write_u32(RTC_CNTL, 45_296 & 0xFFFF).unwrap();
    bus.write_u32(RTC_CRL, 0).unwrap();
    let start = bus.current_cycle;
    assert_eq!(cnt(bus), 45_296);
    bus.set_current_cycle(start + 72_000_000 - 3_000);
    assert_eq!(cnt(bus), 45_296, "just under one second");
    bus.set_current_cycle(start + 72_000_000);
    assert_eq!(cnt(bus), 45_297);
    bus.set_current_cycle(start + 3 * 72_000_000);
    assert_eq!(cnt(bus), 45_299);
}

/// SECIE and ALRIE through the shipped chip config: the scheduler event at
/// each counter second pends RTC_IRQn (3) and, at the alarm match,
/// RTC_Alarm_IRQn (41). The NVIC lines stay disabled, so the pends stay
/// visible in ISPR.
#[cfg(feature = "event-scheduler")]
#[test]
fn second_and_alarm_interrupts_pend_their_nvic_lines() {
    const RTC_CRH: u64 = 0x4000_2800;
    const RTC_ALRH: u64 = 0x4000_2820;
    const RTC_ALRL: u64 = 0x4000_2824;
    const NVIC_ISPR0: u64 = 0xE000_E200;
    const NVIC_ICPR0: u64 = 0xE000_E280;
    const NVIC_ISPR1: u64 = 0xE000_E204;
    const SECF: u32 = 1 << 0;
    const ALRF: u32 = 1 << 1;
    let mut machine = idle_machine();
    {
        let bus = &mut machine.bus;
        enable_rtc_clock(bus);
        // PRL = 7: one counter second is 8 RTCCLK ticks (~17.6 k cycles).
        bus.write_u32(RTC_CRL, CNF).unwrap();
        bus.write_u32(RTC_PRLL, 7).unwrap();
        bus.write_u32(RTC_CNTH, 0).unwrap();
        bus.write_u32(RTC_CNTL, 10).unwrap();
        bus.write_u32(RTC_ALRH, 0).unwrap();
        bus.write_u32(RTC_ALRL, 12).unwrap();
        bus.write_u32(RTC_CRL, RSF).unwrap();
        bus.write_u32(RTC_CRH, 0b011).unwrap(); // SECIE | ALRIE
    }
    let step_until = |machine: &mut Machine<_>, count: u32| {
        let mut steps = 0;
        while cnt(&mut machine.bus) < count {
            machine.step().unwrap();
            steps += 1;
            assert!(steps < 1_000_000, "counter stuck below {count}");
        }
        // Let the scheduler drain the event at the edge.
        for _ in 0..64 {
            machine.step().unwrap();
        }
    };
    step_until(&mut machine, 11);
    let bus = &mut machine.bus;
    assert_ne!(
        bus.read_u32(NVIC_ISPR0).unwrap() & (1 << 3),
        0,
        "RTC_IRQn on SECF"
    );
    assert_eq!(
        bus.read_u32(NVIC_ISPR1).unwrap() & (1 << 9),
        0,
        "no alarm at CNT 11"
    );
    // The handler: clear SECF and the pend.
    bus.write_u32(RTC_CRL, RSF | ALRF).unwrap();
    bus.write_u32(NVIC_ICPR0, 1 << 3).unwrap();
    assert_eq!(bus.read_u32(RTC_CRL).unwrap() & SECF, 0);
    step_until(&mut machine, 12);
    let bus = &mut machine.bus;
    assert_ne!(bus.read_u32(RTC_CRL).unwrap() & ALRF, 0, "ALRF at CNT 12");
    assert_ne!(
        bus.read_u32(NVIC_ISPR1).unwrap() & (1 << 9),
        0,
        "RTC_Alarm_IRQn"
    );
    assert_ne!(
        bus.read_u32(NVIC_ISPR0).unwrap() & (1 << 3),
        0,
        "RTC_IRQn again"
    );
}
