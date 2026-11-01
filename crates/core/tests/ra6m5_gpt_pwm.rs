// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA6M5 GPT1 drives GTIOC1A on P405 and its overflow reaches the core
//! through the ICU. Built from the chip YAML and examples/ek-ra6m5/
//! system.yaml, so the `ra_gpt` wiring, its `gtioc_pads` table and the
//! PmnPFS PMR/PSEL hand-over to the port are under test. The registers are
//! written in the order FSP `R_GPT_Open` + Zephyr `pwm_renesas_ra_set_cycles`
//! write them; the pad is read the way the logic analyzer reads it.
//!
//! GPT1 counts PCLKD (100 MHz) under a 200 MHz bus: two cycles per count.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::logic_capture::LogicSource;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const GPT1: u64 = 0x4016_9100;
const GTSTR: u64 = 0x04;
const GTCLR: u64 = 0x0C;
const GTSSR: u64 = 0x10;
const GTPSR: u64 = 0x14;
const GTCSR: u64 = 0x18;
const GTCR: u64 = 0x2C;
const GTUDDTYC: u64 = 0x30;
const GTIOR: u64 = 0x34;
const GTBER: u64 = 0x40;
const GTCCRC: u64 = 0x54;
const GTPR: u64 = 0x64;
const GTPBR: u64 = 0x68;

const PFS: u64 = 0x4008_0800;
const PWPR: u64 = PFS + 0x503;
/// P405PFS.
const P405PFS: u64 = PFS + 0x40 * 4 + 4 * 5;
const PMR: u32 = 1 << 16;
const PSEL_GPT: u32 = 0b00011 << 24;
const PSEL_SCI: u32 = 0b00101 << 24;

const IELSR0: u64 = 0x4000_6300;
const IR: u32 = 1 << 16;
/// ELC_EVENT_GPT1_COUNTER_OVERFLOW (FSP ra6m5 bsp_elc.h).
const GPT1_OVF: u32 = 0x0CF;
/// Zephyr's ek_ra6m5 pwm1 `overflow` line.
const LINE: u64 = 64;

const IDLE: u32 = 0x2000_2000;
const PERIOD: u32 = 100;
const DUTY: u32 = 30;
const CYCLES_PER_COUNT: u64 = 2;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/r7fa6m5bh.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("r7fa6m5bh chip");
    let mut manifest =
        SystemManifest::from_file(root("examples/ek-ra6m5/system.yaml")).expect("ek-ra6m5");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(IDLE), 0xE7FE).unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2000_7F00);
    m
}

/// `R_BSP_PinAccessEnable` + a PmnPFS store + `R_BSP_PinAccessDisable`.
fn pfs(m: &mut Machine<CortexM>, value: u32) {
    m.bus.write_u8(PWPR, 0).unwrap();
    m.bus.write_u8(PWPR, 1 << 6).unwrap();
    m.bus.write_u32(P405PFS, value).unwrap();
    m.bus.write_u8(PWPR, 0).unwrap();
    m.bus.write_u8(PWPR, 1 << 7).unwrap();
}

/// FSP `R_GPT_Open` (saw-wave PWM, GTIOCA high at the cycle end, low at
/// compare match, initial and stop level low), then Zephyr's `pwm_set`:
/// stop, `R_GPT_PeriodSet`, `R_GPT_DutyCycleSet`, start.
fn zephyr_pwm(m: &mut Machine<CortexM>, period: u32, duty: u32) {
    let w = |m: &mut Machine<CortexM>, off: u64, v: u32| m.bus.write_u32(GPT1 + off, v).unwrap();
    w(m, GTCR, 0);
    w(m, GTSSR, 1 << 31);
    w(m, GTPSR, 1 << 31);
    w(m, GTCSR, 1 << 31);
    w(m, GTPBR, period - 1);
    w(m, GTPR, period - 1);
    w(m, GTCCRC, period / 2 - 1);
    w(m, GTCLR, 1 << 1);
    w(m, GTIOR, (1 << 8) | 0b01001);
    w(m, GTUDDTYC, 3);
    w(m, GTUDDTYC, 1);
    w(m, GTBER, 0x55_0000);
    w(m, GTCLR, 1 << 1);
    w(m, GTCCRC, duty - 1);
    w(m, GTSTR, 1 << 1);
}

fn pad(m: &mut Machine<CortexM>) -> Option<bool> {
    let idx = m.bus.find_peripheral_index_by_name("port4").expect("port4");
    m.logic_watch(&[Some(LogicSource::pad(idx, 5))])[0]
}

/// Step for `cycles` bus cycles and return `(cycles high, edges)` on P405.
fn sample(m: &mut Machine<CortexM>, cycles: u64) -> (u64, u64) {
    let end = m.total_cycles + cycles;
    let (mut high, mut edges) = (0, 0);
    let mut last = pad(m);
    while m.total_cycles < end {
        let before = m.total_cycles;
        m.step().unwrap();
        let level = pad(m);
        if level == Some(true) {
            high += m.total_cycles - before;
        }
        if level != last {
            edges += 1;
            last = level;
        }
    }
    (high, edges)
}

#[test]
fn gtioc1a_reaches_p405_only_while_pmnpfs_selects_the_gpt() {
    let mut m = machine();
    zephyr_pwm(&mut m, PERIOD, DUTY);
    let period = u64::from(PERIOD) * CYCLES_PER_COUNT;
    let (_, edges) = sample(&mut m, 4 * period);
    assert_eq!(edges, 0, "P405 is still a port pin: no PWM on it");
    pfs(&mut m, PMR | PSEL_SCI);
    let (_, edges) = sample(&mut m, 4 * period);
    assert_eq!(edges, 0, "PSEL names SCI, not the GPT");
    pfs(&mut m, PMR | PSEL_GPT);
    let (_, edges) = sample(&mut m, 10 * period);
    assert!(
        (19..=21).contains(&edges),
        "ten periods, two edges each: {edges}"
    );
    pfs(&mut m, 0);
    let (_, edges) = sample(&mut m, 4 * period);
    assert!(edges <= 1, "handed back to the port: {edges}");
}

#[test]
fn p405_is_high_for_gtccra_plus_one_counts_of_each_period() {
    let mut m = machine();
    pfs(&mut m, PMR | PSEL_GPT);
    zephyr_pwm(&mut m, PERIOD, DUTY);
    let period = u64::from(PERIOD) * CYCLES_PER_COUNT;
    // First period: the initial level (low) until the first cycle end.
    let (high, _) = sample(&mut m, period - 8);
    assert_eq!(high, 0, "first period shows the initial level");
    sample(&mut m, 8);
    let (high, edges) = sample(&mut m, 20 * period);
    let want = 20 * u64::from(DUTY) * CYCLES_PER_COUNT;
    assert!(
        high.abs_diff(want) <= 40,
        "high {high} cycles of {} (want {want})",
        20 * period
    );
    assert!((39..=41).contains(&edges), "{edges} edges in 20 periods");
}

#[test]
fn gpt1_overflow_latches_its_ielsr_line_once_per_period() {
    let mut m = machine();
    m.bus.write_u32(IELSR0 + 4 * LINE, GPT1_OVF).unwrap();
    zephyr_pwm(&mut m, PERIOD, DUTY);
    let period = u64::from(PERIOD) * CYCLES_PER_COUNT;
    let start = m.total_cycles;
    let mut hits = Vec::new();
    while m.total_cycles < start + 5 * period + 8 {
        m.step().unwrap();
        let reg = IELSR0 + 4 * LINE;
        if m.bus.read_u32(reg).unwrap() & IR != 0 {
            hits.push(m.total_cycles - start);
            m.bus.write_u32(reg, GPT1_OVF).unwrap();
        }
    }
    assert_eq!(hits.len(), 5, "one overflow per period: {hits:?}");
    for (k, at) in hits.iter().enumerate() {
        let want = (k as u64 + 1) * period;
        assert!(at.abs_diff(want) <= 8, "overflow {k} at {at}, want {want}");
    }
}
