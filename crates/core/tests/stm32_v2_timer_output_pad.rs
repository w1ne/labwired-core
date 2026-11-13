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

/// Native complementary output routes from STM32L476 DS10198 table 17;
/// output-stage timing from RM0351 section 30.3.15. These checks use the same
/// Machine logic edge ring that supplies the browser instruments.
mod l476_complementary {
    use super::*;
    use labwired_core::logic_capture::LogicEdge;

    const TIM1: u64 = 0x4001_2C00;
    const GPIOB: u64 = 0x4800_0400;
    const BDTR: u64 = 0x44;
    const MOE: u32 = 1 << 15;

    fn capture(af: u32, ccer: u32, bdtr: u32, ccr: u32, mode: u32) -> (Vec<LogicEdge>, bool) {
        let mut m = machine(L476.system);
        set_bits(&mut m, 0x4002_104C, 3); // GPIOA/B AHB2 clocks.
        set_bits(&mut m, 0x4002_1060, 1 << 11); // TIM1 APB2 clock.
        for (base, pin, function) in [(L476.gpioa, 8u32, 1u32), (GPIOB, 13, af)] {
            let moder = m.bus.read_u32(base).unwrap();
            m.bus
                .write_u32(base, (moder & !(3 << (pin * 2))) | (2 << (pin * 2)))
                .unwrap();
            let offset = base + 0x24;
            let shift = (pin - 8) * 4;
            let afr = m.bus.read_u32(offset).unwrap();
            m.bus
                .write_u32(offset, (afr & !(15 << shift)) | (function << shift))
                .unwrap();
        }
        for (offset, value) in [
            (TIM_PSC, 0),
            (TIM_ARR, 199),
            (TIM_CCR1, ccr),
            (TIM_CCMR1, mode << 4),
            (TIM_CCER, ccer),
            (BDTR, bdtr),
            (TIM_EGR, 1),
            (TIM_CR1, 1),
        ] {
            m.bus.write_u32(TIM1 + offset, value).unwrap();
        }
        let a = m.bus.find_peripheral_index_by_name("gpioa").unwrap();
        let b = m.bus.find_peripheral_index_by_name("gpiob").unwrap();
        m.logic_watch(&[
            Some(LogicSource::Pad {
                peripheral: a,
                pin: 8,
            }),
            Some(LogicSource::Pad {
                peripheral: b,
                pin: 13,
            }),
        ]);
        for _ in 0..2_000 {
            m.step().unwrap();
            if af == 1 && ccer & 4 != 0 && bdtr & MOE != 0 {
                let pad = m
                    .read_logic_level(LogicSource::Pad {
                        peripheral: b,
                        pin: 13,
                    })
                    .expect("enabled PB13 AF output has a native pad level");
                assert_eq!(
                    m.bus.read_u32(GPIOB + 0x10).unwrap() & (1 << 13) != 0,
                    pad,
                    "firmware IDR and native PB13 capture must agree"
                );
            }
        }
        let n_level = m.bus.read_u32(GPIOB + 0x10).unwrap() & (1 << 13) != 0;
        (m.logic_read_edges(0).edges, n_level)
    }

    #[test]
    fn l476_tim1_pb14_armed_undriven_records_later_native_states() {
        use labwired_core::logic_capture::{
            attach_logic_states, build_logic_edges_result, LogicChannelMeta, PadState,
        };

        let mut m = machine(L476.system);
        set_bits(&mut m, 0x4002_104C, 2); // GPIOB AHB2 clock.
        set_bits(&mut m, 0x4002_1060, 1 << 11); // TIM1 APB2 clock.
        let moder = m.bus.read_u32(GPIOB).unwrap();
        m.bus
            .write_u32(GPIOB, (moder & !(3 << 28)) | (2 << 28))
            .unwrap();
        let afr = m.bus.read_u32(GPIOB + 0x24).unwrap();
        m.bus
            .write_u32(GPIOB + 0x24, (afr & !(15 << 24)) | (1 << 24))
            .unwrap();
        for (offset, value) in [
            (TIM_PSC, 0),
            (TIM_ARR, 199),
            (TIM_CCR1 + 4, 80),   // CCR2.
            (TIM_CCMR1, 6 << 12), // OC2M PWM1, immediate compare.
            (TIM_CCER, 0),        // The observed output is genuinely disabled at arm.
            (BDTR, MOE | 2),
            (TIM_EGR, 1),
            (TIM_CR1, 1),
        ] {
            m.bus.write_u32(TIM1 + offset, value).unwrap();
        }
        let b = m.bus.find_peripheral_index_by_name("gpiob").unwrap();
        let initial = m.logic_watch(&[Some(LogicSource::pad(b, 14))]);
        let initial_states = m.logic_initial_states().to_vec();
        assert_eq!(initial, vec![None]);
        assert_eq!(initial_states, vec![None]);
        let armed_at = m.logic_now_cycle();
        m.bus.write_u32(TIM1 + TIM_CCER, 1 << 6).unwrap(); // CC2NE.
        for _ in 0..400 {
            m.step().unwrap();
        }
        m.bus.write_u32(TIM1 + TIM_CCER, 0).unwrap();
        for _ in 0..200 {
            m.step().unwrap();
        }
        m.bus.write_u32(TIM1 + TIM_CCR1 + 4, 0).unwrap();
        m.bus.write_u32(TIM1 + TIM_CCER, 1 << 6).unwrap();
        for _ in 0..20 {
            m.step().unwrap();
        }

        let edges = m.logic_read_edges(0);
        let states = m.logic_read_states(0);
        assert_eq!(edges.dropped, 0);
        assert_eq!(states.dropped, 0);
        assert!(edges.edges.iter().any(|edge| edge.value));
        assert!(edges.edges.iter().any(|edge| !edge.value));
        for expected in [PadState::High, PadState::HighZ, PadState::Low] {
            assert!(
                states.edges.iter().any(|edge| edge.state == expected),
                "{expected:?}"
            );
        }
        assert!(states
            .edges
            .iter()
            .all(|edge| edge.cycle >= armed_at && edge.cycle <= m.logic_now_cycle()));
        let meta = [LogicChannelMeta {
            ch: 0,
            peripheral: "gpiob".into(),
            pin: 14,
            initial: initial[0],
        }];
        let mut exported = build_logic_edges_result(&meta, &edges, m.logic_now_cycle());
        attach_logic_states(&mut exported, &initial_states, &states);
        assert_eq!(exported.channels[0].initial_state, None);
        assert_eq!(
            exported.channels[0].states.as_ref().unwrap().len(),
            states.edges.len()
        );
    }

    fn assert_edges(edges: &[LogicEdge], channel: u32, expected: &[(u64, bool)]) {
        let actual: Vec<_> = edges
            .iter()
            .filter(|e| e.ch == channel && e.cycle >= 200)
            .collect();
        assert!(
            actual.len() >= 10,
            "channel {channel} has no native PWM edges: {actual:?}"
        );
        for edge in actual {
            assert!(
                expected.contains(&(edge.cycle % 200, edge.value)),
                "unexpected native edge {edge:?}"
            );
        }
    }

    #[test]
    fn l476_tim1_n_only_follows_ocref_on_pb13() {
        let (edges, _) = capture(1, 4, MOE | 2, 80, 6);
        assert_edges(&edges, 1, &[(0, true), (80, false)]);
        assert!(edges.iter().filter(|e| e.ch == 0).count() <= 1);
        let (full, high) = capture(1, 4, MOE | 2, 200, 6);
        assert!(high, "CCR=ARR+1 keeps the N-only commutation gate active");
        assert!(full.iter().filter(|e| e.ch == 1).count() <= 1);
    }

    #[test]
    fn l476_tim1_both_enabled_have_exact_two_cycle_dead_time() {
        let (edges, _) = capture(1, 5, MOE | 2, 80, 6);
        assert_edges(&edges, 0, &[(2, true), (80, false)]);
        assert_edges(&edges, 1, &[(82, true), (0, false)]);
        let mut levels = [false; 2];
        for edge in edges {
            levels[edge.ch as usize] = edge.value;
            assert!(
                !(levels[0] && levels[1]),
                "overlap at native cycle {}",
                edge.cycle
            );
        }
    }

    #[test]
    fn l476_tim1_complementary_route_requires_af_enable_and_moe() {
        for (af, enable, bdtr) in [(2, 4, MOE), (1, 0, MOE), (1, 4, 0)] {
            let (edges, high) = capture(af, enable, bdtr, 80, 6);
            assert!(
                edges.iter().filter(|e| e.ch == 1).count() <= 1,
                "false route for AF={af}, CCER={enable}, BDTR={bdtr}"
            );
            assert!(
                !high,
                "an inactive AF driver must not override the reset IDR input level"
            );
        }
    }

    #[test]
    fn l476_tim1_pwm2_and_n_polarity_are_native() {
        let (edges, _) = capture(1, 4, MOE | 2, 80, 7);
        assert_edges(&edges, 1, &[(80, true), (0, false)]);
        let (inverted, _) = capture(1, 12, MOE | 2, 80, 6);
        assert_edges(&inverted, 1, &[(80, true), (0, false)]);
    }
}
