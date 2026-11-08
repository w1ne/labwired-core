// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32 V2-port timer OUTPUTS reach the pad: TIM2_CH1 PWM, as stm32duino's
//! `analogWrite(PA0, …)` configures it, shows up as edges on the pad for the
//! F411 (F4 AF table, AF1), G071 / G0B1 (G0 table, AF2), G474 (G4 table, AF1),
//! L073 (L0 table, AF2), L476 (L4), WB55 (WB), U575 / U545 (U5) and H563 (H5),
//! all AF1, and WBA52 (WBA table, TIM2_CH1 on PA5 AF1: PA0 is not TIM2
//! there).
//!
//! Before this, only F1 ports had a timer output route
//! (`SystemBus::wire_stm32_timer_output_pads`), so the Arduino matrix L6 cell
//! on these Nucleo / Black Pill rows passed on the UART marker alone with the
//! pad showing 0-1 edges.
//!
//! Each chip also carries the two controls that make the pass mean something:
//! the same timer with the pad on the WRONG alternate function (the AF the
//! other family table uses), and the right AF with `CC1E = 0`. Both must stay
//! dark.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::logic_capture::LogicSource;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

type Cm = Machine<CortexM>;

const SRAM_BASE: u64 = 0x2000_0000;
const TIM2_BASE: u64 = 0x4000_0000;

// TIMx register offsets (RM0383 §13.4 / RM0444 §22.4 / RM0440 §29.6 agree).
const TIM_CR1: u64 = 0x00;
const TIM_EGR: u64 = 0x14;
const TIM_CCMR1: u64 = 0x18;
const TIM_CCER: u64 = 0x20;
const TIM_PSC: u64 = 0x28;
const TIM_ARR: u64 = 0x2C;
const TIM_CCR1: u64 = 0x34;

/// One chip under test: the Arduino-matrix system it boots in, where its RCC
/// enables GPIOA and TIM2, and which GPIOA pin and AF nibble its datasheet
/// gives TIM2_CH1 (stm32duino `PinMap_TIM`).
struct Part {
    system: &'static str,
    gpioa: u64,
    pin: u32,
    /// (RCC register address, bit) for GPIOA, then TIM2.
    gpio_en: (u64, u32),
    tim_en: (u64, u32),
    af: u32,
    /// An AF that is NOT TIM2 on this pad for this part — the other table's.
    wrong_af: u32,
}

const F411: Part = Part {
    system: "validation/arduino-matrix/systems/stm32f411ceu6.yaml",
    gpioa: 0x4002_0000,
    pin: 0,
    gpio_en: (0x4002_3800 + 0x30, 0), // RCC_AHB1ENR.GPIOAEN
    tim_en: (0x4002_3800 + 0x40, 0),  // RCC_APB1ENR.TIM2EN
    af: 1,
    wrong_af: 2, // TIM5_CH1 on an F411, not TIM2
};

const G071: Part = Part {
    system: "validation/arduino-matrix/systems/stm32g071rb.yaml",
    gpioa: 0x5000_0000,
    pin: 0,
    gpio_en: (0x4002_1000 + 0x34, 0), // RCC_IOPENR.GPIOAEN
    tim_en: (0x4002_1000 + 0x3C, 0),  // RCC_APBENR1.TIM2EN
    af: 2,
    wrong_af: 1, // the F4/G4 TIM2 number, not TIM2 on a G0
};

const G0B1: Part = Part {
    system: "validation/arduino-matrix/systems/stm32g0b1re.yaml",
    ..G071
};

const G474: Part = Part {
    system: "validation/arduino-matrix/systems/stm32g474re.yaml",
    gpioa: 0x4800_0000,
    pin: 0,
    gpio_en: (0x4002_1000 + 0x4C, 0), // RCC_AHB2ENR.GPIOAEN
    tim_en: (0x4002_1000 + 0x58, 0),  // RCC_APB1ENR1.TIM2EN
    af: 1,
    wrong_af: 2, // the G0 TIM2 number (TIM5 on a G4)
};

const L073: Part = Part {
    system: "validation/arduino-matrix/systems/stm32l073.yaml",
    gpioa: 0x5000_0000,
    pin: 0,
    gpio_en: (0x4002_1000 + 0x2C, 0), // RCC_IOPENR.IOPAEN
    tim_en: (0x4002_1000 + 0x38, 0),  // RCC_APB1ENR.TIM2EN
    af: 2,
    wrong_af: 1, // the L4/F4 TIM2 number, not TIM2 on an L0
};

const L476: Part = Part {
    system: "validation/arduino-matrix/systems/stm32l476.yaml",
    gpioa: 0x4800_0000,
    pin: 0,
    gpio_en: (0x4002_1000 + 0x4C, 0), // RCC_AHB2ENR.GPIOAEN
    tim_en: (0x4002_1000 + 0x58, 0),  // RCC_APB1ENR1.TIM2EN
    af: 1,
    wrong_af: 2, // TIM5_CH1 on an L476, not TIM2
};

const WB55: Part = Part {
    system: "validation/arduino-matrix/systems/stm32wb55.yaml",
    gpioa: 0x4800_0000,
    pin: 0,
    gpio_en: (0x5800_0000 + 0x4C, 0), // RCC_AHB2ENR.GPIOAEN
    tim_en: (0x5800_0000 + 0x58, 0),  // RCC_APB1ENR1.TIM2EN
    af: 1,
    wrong_af: 2, // the L0/G0 TIM2 number, not TIM2 on a WB
};

const U575: Part = Part {
    system: "validation/arduino-matrix/systems/stm32u575.yaml",
    gpioa: 0x4202_0000,
    pin: 0,
    gpio_en: (0x4602_0C00 + 0x8C, 0), // RCC_AHB2ENR1.GPIOAEN
    tim_en: (0x4602_0C00 + 0x9C, 0),  // RCC_APB1ENR1.TIM2EN
    af: 1,
    wrong_af: 2, // TIM5_CH1 on a U5, not TIM2
};

const U545: Part = Part {
    system: "validation/arduino-matrix/systems/stm32u545.yaml",
    ..U575
};

const H563: Part = Part {
    system: "validation/arduino-matrix/systems/stm32h563.yaml",
    gpioa: 0x4202_0000,
    pin: 0,
    gpio_en: (0x4402_0C00 + 0x8C, 0), // RCC_AHB2ENR.GPIOAEN
    tim_en: (0x4402_0C00 + 0x9C, 0),  // RCC_APB1LENR.TIM2EN
    af: 1,
    wrong_af: 2, // TIM5_CH1 on an H5, not TIM2
};

const WBA52: Part = Part {
    system: "validation/arduino-matrix/systems/stm32wba52.yaml",
    gpioa: 0x4202_0000,
    pin: 5,
    gpio_en: (0x4602_0C00 + 0x8C, 0), // RCC_AHB2ENR.GPIOAEN
    tim_en: (0x4602_0C00 + 0x9C, 0),  // RCC_APB1ENR1.TIM2EN
    af: 1,
    wrong_af: 2, // TIM3 on a WBA, and PA5 is not a TIM3 pad
};

fn machine(relative_system_yaml: &str) -> Cm {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative_system_yaml);
    let manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|e| panic!("load {relative_system_yaml}: {e}"));
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    // A NOP slab in SRAM ending in a branch back to its start, so `step()`
    // advances cycles without a firmware image.
    for i in 0..1022u64 {
        let byte = if i % 2 == 0 { 0x00 } else { 0x20 };
        machine.bus.write_u8(SRAM_BASE + i, byte).unwrap();
    }
    machine.bus.write_u8(SRAM_BASE + 1022, 0xFF).unwrap();
    machine.bus.write_u8(SRAM_BASE + 1023, 0xE5).unwrap();
    machine.cpu.pc = SRAM_BASE as u32;
    machine
}

fn set_bits(machine: &mut Cm, addr: u64, bits: u32) {
    let old = machine.bus.read_u32(addr).unwrap();
    machine.bus.write_u32(addr, old | bits).unwrap();
}

/// stm32duino's `analogWrite(PA0, 128)` path, register for register: GPIOA
/// and TIM2 clocks, the pin's MODER = AF with its AFRL nibble = `af`, then
/// TIM2 CH1 PWM mode 1 at 50 % duty and the counter started. Returns the pad
/// edges seen over 4 000 cycles (a 100-cycle period, so ~80 on a routed pad).
fn tim2_ch1_pwm_edges(part: &Part, af: u32, cc1e: bool) -> usize {
    let mut m = machine(part.system);
    set_bits(&mut m, part.gpio_en.0, 1 << part.gpio_en.1);
    set_bits(&mut m, part.tim_en.0, 1 << part.tim_en.1);

    let pin = part.pin;
    let moder = m.bus.read_u32(part.gpioa).unwrap();
    m.bus
        .write_u32(
            part.gpioa,
            (moder & !(0b11 << (2 * pin))) | (0b10 << (2 * pin)),
        )
        .unwrap();
    let afrl = m.bus.read_u32(part.gpioa + 0x20).unwrap();
    m.bus
        .write_u32(
            part.gpioa + 0x20,
            (afrl & !(0xF << (4 * pin))) | (af << (4 * pin)),
        )
        .unwrap();

    let tim = |off| TIM2_BASE + off;
    m.bus.write_u32(tim(TIM_PSC), 0).unwrap();
    m.bus.write_u32(tim(TIM_ARR), 99).unwrap();
    m.bus.write_u32(tim(TIM_CCR1), 50).unwrap();
    // OC1M = 110 (PWM mode 1), OC1PE.
    m.bus
        .write_u32(tim(TIM_CCMR1), (0b110 << 4) | (1 << 3))
        .unwrap();
    m.bus.write_u32(tim(TIM_CCER), u32::from(cc1e)).unwrap();
    m.bus.write_u32(tim(TIM_EGR), 1).unwrap();
    m.bus.write_u32(tim(TIM_CR1), 1).unwrap();

    let gpioa = m
        .bus
        .find_peripheral_index_by_name("gpioa")
        .expect("gpioa on the bus");
    m.logic_watch(&[Some(LogicSource::Pad {
        peripheral: gpioa,
        pin: pin as u8,
    })]);
    for _ in 0..4_000 {
        m.step().expect("step");
    }
    m.logic_read_edges(0).edges.len()
}

fn assert_routed(name: &str, part: &Part) {
    let on = tim2_ch1_pwm_edges(part, part.af, true);
    assert!(
        on >= 20,
        "{name}: TIM2_CH1 PWM on PA{} AF{} should toggle the pad, saw {on} edges",
        part.pin,
        part.af
    );
    let wrong = tim2_ch1_pwm_edges(part, part.wrong_af, true);
    assert!(
        wrong <= 1,
        "{name}: PA{} on AF{} is not TIM2_CH1, but the pad saw {wrong} edges",
        part.pin,
        part.wrong_af
    );
    let off = tim2_ch1_pwm_edges(part, part.af, false);
    assert!(
        off <= 1,
        "{name}: CC1E = 0 leaves the output disabled, but the pad saw {off} edges"
    );
}

#[test]
fn f411_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32F411", &F411);
}

#[test]
fn g071_tim2_ch1_pwm_reaches_pa0_on_af2() {
    assert_routed("STM32G071", &G071);
}

#[test]
fn g0b1_tim2_ch1_pwm_reaches_pa0_on_af2() {
    assert_routed("STM32G0B1", &G0B1);
}

#[test]
fn g474_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32G474", &G474);
}

#[test]
fn l073_tim2_ch1_pwm_reaches_pa0_on_af2() {
    assert_routed("STM32L073", &L073);
}

#[test]
fn l476_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32L476", &L476);
}

#[test]
fn wb55_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32WB55", &WB55);
}

#[test]
fn u575_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32U575", &U575);
}

#[test]
fn u545_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32U545", &U545);
}

#[test]
fn h563_tim2_ch1_pwm_reaches_pa0_on_af1() {
    assert_routed("STM32H563", &H563);
}

#[test]
fn wba52_tim2_ch1_pwm_reaches_pa5_on_af1() {
    assert_routed("STM32WBA52", &WBA52);
}

#[test]
fn unknown_timer_pad_map_is_refused() {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(G071.system);
    let manifest = SystemManifest::from_file(&system_path).unwrap();
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let mut chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let tim2 = chip
        .peripherals
        .iter_mut()
        .find(|p| p.id == "tim2")
        .expect("tim2");
    tim2.config.insert(
        "pad_map".into(),
        serde_yaml::Value::String("stm32f7".into()),
    );
    let err = SystemBus::from_config(&chip, &manifest)
        .err()
        .expect("an unknown pad_map must be refused");
    assert!(format!("{err:#}").contains("unknown pad_map"), "{err:#}");
}
