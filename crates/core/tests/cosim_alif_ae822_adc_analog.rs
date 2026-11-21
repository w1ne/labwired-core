// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the ADC12 instances
//! (`alif_adc12`, alif_ensemble-cmsis-dfp AE822FA0E5597 soc.h) on the
//! onboarding Alif AE822 through the descriptor's `analog_pins` map
//! (Zephyr ensemble-e8-appkit-pinctrl.dtsi: ANA_S0..S5 on P0_0..P0_5 are
//! ADC120 channels 0..5, ANA_S6..S11 on P0_6..P1_3 are ADC121 channels
//! 0..5). The NTC is on P0_3 (ADC120 channel 3) and the diode on P1_2
//! (ADC121 channel 4), read three ways:
//! - Zephyr `adc_alif.c` single shot with the e1.dtsi defaults (256
//!   accumulated samples, shift 8 right): masks `~BIT(ch) & 0x1FF`, initial
//!   channel, `START_SRC.ENA`, `ADC_CONTROL` bit 0, the DONE1 ISR reads
//!   `ADC_SAMPLE_REG_[ADC_SEL]`;
//! - CMSIS `Driver_ADC.c` continuous rotate scan on ADC120 over the NTC and
//!   an idle pad, the DONE0 ISR reading the channel it prefetched from
//!   `ADC_SEL`, while the NTC warms;
//! - the threshold comparator: CMPA (below A) fires on the idle pad (0) and,
//!   once the NTC warms, on the NTC channel too.
//!
//! The reference is 1.8 V (an assumption: no public Ensemble datasheet
//! value), so the -10 °C divider (2.79 V) saturates. Also: the netlists moved
//! to P0_0 and P1_4 (ADC122 channel 0) convert there while P0_3 and P1_2 go
//! back to 0.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

const ADC120: u64 = 0x4902_0000;
const ADC121: u64 = 0x4902_1000;
const ADC122: u64 = 0x4902_2000;
const START_SRC: u64 = 0x00;
const THRESH_A: u64 = 0x04;
const THRESH_B: u64 = 0x08;
const CLK_DIVISOR: u64 = 0x0C;
const INTERRUPT: u64 = 0x10;
const INTERRUPT_MASK: u64 = 0x14;
const SAMPLE_WIDTH: u64 = 0x18;
const AVG_NUM: u64 = 0x20;
const SHIFT_CONTROL: u64 = 0x24;
const CONTROL: u64 = 0x30;
const SEQUENCER_CTRL: u64 = 0x34;
const SEL: u64 = 0x3C;
const SAMPLE_REG0: u64 = 0x50;
const START_CONT: u32 = 1 << 6;
const START_ENA: u32 = 1 << 7;
const SINGLE_SHOT: u32 = 1 << 0;
const BUSY: u32 = 1 << 1;
const DONE0: u32 = 1 << 0;
const DONE1: u32 = 1 << 1;
const CMPA: u32 = 1 << 2;
const FIXED_SCAN: u32 = 1 << 16;
const SHIFT_RIGHT: u32 = 1 << 16;
const MASK_ALL_CHANNELS: u32 = 0x1FF;
const INIT_POS: u32 = 12;
/// The profile's `vref_mv`.
const VREF_MV: u32 = 1800;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn ntc_volts(temperature_c: f64) -> f64 {
    let r = ntc_resistance_ohms(temperature_c);
    3.3 * r / (10_000.0 + r)
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn millivolts(volts: f64) -> u32 {
    (volts * 1000.0).round() as u32
}

/// 12-bit, (2^12 - 1) / VREF, truncated, clamped.
fn code(volts: f64) -> u32 {
    (millivolts(volts) * 4095 / VREF_MV).min(4095)
}

/// The onboarding AE822 with the generic NTC and diode netlists routed to
/// the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/alif_ae822fa0e5597bs0.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load AE822");
    let analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic Beta NTC and Shockley diode netlists");
    profile.cosim_models = analog.cosim_models;
    for model in &mut profile.cosim_models {
        let (key, pad) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", ntc_pad),
            "diode_load_line" => ("v_diode", diode_pad),
            other => panic!("unexpected model {other}"),
        };
        model
            .outputs
            .insert(key.to_string(), format!("board.analog.{pad}_volts"));
    }
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    profile.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&chip_path).expect("load AE822 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build AE822 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC12 channels through analog_pins"
    );
    (bus, session)
}

/// Move the NTC and solve both netlists to the next boundary.
fn settle(session: &mut CosimSession, bus: &mut SystemBus, cycles: &mut u64, temperature_c: f64) {
    session
        .set_signal_number("ui.ntc.temperature_c", temperature_c)
        .expect("set NTC temperature");
    *cycles += session.cycles_until_boundary(*cycles);
    let (steps, errors) = session
        .advance_to(*cycles, bus)
        .expect("solve and route both netlists");
    assert_eq!(steps.len(), 2, "both netlists solve each step");
    assert!(errors.is_empty(), "route errors: {errors:?}");
}

fn adc_irq(bus: &SystemBus, id: &str) -> bool {
    bus.peripherals
        .iter()
        .find(|p| p.name == id)
        .unwrap_or_else(|| panic!("{id} on the bus"))
        .dev
        .irq_line_level()
        .expect("alif_adc12 reports its request")
}

fn sample(bus: &SystemBus, adc: u64, channel: u32) -> u32 {
    bus.read_u32(adc + SAMPLE_REG0 + 4 * u64::from(channel))
        .unwrap()
}

fn modify(bus: &mut SystemBus, addr: u64, clear: u32, set: u32) {
    let value = bus.read_u32(addr).unwrap();
    bus.write_u32(addr, (value & !clear) | set).unwrap();
}

/// Zephyr `adc_hw_init` with the e1.dtsi defaults: sample width 16, clock
/// divisor 2, 256 samples, shift 8 right, thresholds 1, fixed scan.
fn zephyr_init(bus: &mut SystemBus, adc: u64) {
    modify(bus, adc + SAMPLE_WIDTH, 0xFFFF, 16);
    bus.write_u32(adc + CLK_DIVISOR, 2).unwrap();
    bus.write_u32(adc + AVG_NUM, 256).unwrap();
    bus.write_u32(adc + SHIFT_CONTROL, 8 | SHIFT_RIGHT).unwrap();
    bus.write_u32(adc + THRESH_A, 1).unwrap();
    bus.write_u32(adc + THRESH_B, 1).unwrap();
    bus.write_u32(adc + CONTROL, 0).unwrap();
    modify(bus, adc + SEQUENCER_CTRL, 0, FIXED_SCAN);
    bus.write_u32(adc + INTERRUPT_MASK, !(DONE0 | DONE1) & 0xF)
        .unwrap();
}

/// Zephyr `adc_channel_setup` + `adc_read`: the DONE1 ISR, then
/// `adc_context_on_complete` clears `ENA` and the start bit.
fn zephyr_read(bus: &mut SystemBus, adc: u64, id: &str, channel: u32) -> u32 {
    modify(
        bus,
        adc + SEQUENCER_CTRL,
        0xF << INIT_POS,
        channel << INIT_POS,
    );
    modify(
        bus,
        adc + SEQUENCER_CTRL,
        MASK_ALL_CHANNELS,
        !(1 << channel) & MASK_ALL_CHANNELS,
    );
    let seq = bus.read_u32(adc + SEQUENCER_CTRL).unwrap();
    assert_eq!(seq & (1 << ((seq & 0xF000) >> 12)), 0, "channel not masked");
    assert_eq!(bus.read_u32(adc + CONTROL).unwrap() & BUSY, 0, "not busy");
    bus.write_u32(adc + INTERRUPT_MASK, !(DONE0 | DONE1) & 0xF)
        .unwrap();
    modify(bus, adc + START_SRC, 0, START_ENA);
    modify(bus, adc + CONTROL, 0, SINGLE_SHOT);
    assert!(
        adc_irq(bus, id),
        "{id} channel {channel}: DONE requests the NVIC"
    );
    bus.write_u32(adc + INTERRUPT, DONE0).unwrap();
    bus.write_u32(adc + INTERRUPT, DONE1).unwrap();
    let sel = bus.read_u32(adc + SEL).unwrap();
    assert_eq!(sel, channel, "ADC_SEL");
    let value = sample(bus, adc, sel);
    // The comparators fire too (thresholds 1); the e1.dtsi line masks them.
    modify(bus, adc + START_SRC, START_ENA, 0);
    modify(bus, adc + CONTROL, SINGLE_SHOT, 0);
    value
}

#[test]
fn ae822_p0_3_and_p1_2_convert_ntc_and_diode_on_two_instances() {
    let (mut bus, mut session) = board("P0_3", "P1_2");
    for adc in [ADC120, ADC121, ADC122] {
        assert_eq!(bus.read_u32(adc + START_SRC).unwrap(), 0, "reset");
        zephyr_init(&mut bus, adc);
    }
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [25.0_f64, 50.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.P0_3_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.P1_2_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        let ntc_code = zephyr_read(&mut bus, ADC120, "adc120", 3);
        assert_eq!(ntc_code, code(ntc_v), "P0_3 at {temperature_c} °C");
        assert!(ntc_code < 4095, "below full scale at {temperature_c} °C");
        assert_eq!(
            zephyr_read(&mut bus, ADC121, "adc121", 4),
            code(diode_v),
            "P1_2"
        );
        // The pads' neighbours on the same and the other instance are idle.
        assert_eq!(zephyr_read(&mut bus, ADC120, "adc120", 4), 0, "P0_4 idle");
        assert_eq!(zephyr_read(&mut bus, ADC121, "adc121", 3), 0, "P1_1 idle");
        assert_eq!(zephyr_read(&mut bus, ADC122, "adc122", 3), 0, "P1_7 idle");
        ntc_codes.push(ntc_code);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    let mv = ntc_codes[0] * VREF_MV / 4095;
    assert!(
        mv.abs_diff(millivolts(ntc_volts(25.0))) <= 1,
        "code * VREF / 4095 gives {mv} mV"
    );
    // At -10 °C the divider (about 2.8 V) is above the 1.8 V reference.
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    assert_eq!(
        zephyr_read(&mut bus, ADC120, "adc120", 3),
        4095,
        "over-range clamps"
    );
}

#[test]
fn ae822_cmsis_continuous_rotate_scan_and_comparator_follow_the_ntc() {
    let (mut bus, mut session) = board("P0_3", "P1_2");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_25 = code(signal_voltage(&session, "board.analog.P0_3_volts"));

    // CMSIS ADC_PowerControl defaults + ARM_ADC_SEQUENCER_MSK_CH_CTRL (all
    // but channels 1 and 3), ARM_ADC_CHANNEL_INIT_VAL 1, multi-channel scan;
    // comparator "below A" with A just under the 25 °C code.
    bus.write_u32(ADC120 + AVG_NUM, 256).unwrap();
    bus.write_u32(ADC120 + SHIFT_CONTROL, 8 | SHIFT_RIGHT)
        .unwrap();
    bus.write_u32(ADC120 + SEQUENCER_CTRL, 0).unwrap();
    modify(
        &mut bus,
        ADC120 + SEQUENCER_CTRL,
        MASK_ALL_CHANNELS,
        MASK_ALL_CHANNELS & !((1 << 1) | (1 << 3)),
    );
    modify(
        &mut bus,
        ADC120 + SEQUENCER_CTRL,
        0xF << INIT_POS,
        1 << INIT_POS,
    );
    bus.write_u32(ADC120 + THRESH_A, ntc_25 - 200).unwrap();
    bus.write_u32(ADC120 + THRESH_B, 4095).unwrap();
    bus.write_u32(ADC120 + CONTROL, 1 << 16).unwrap();
    // ADC_Start: unmask, START_SRC |= CONT.
    bus.write_u32(ADC120 + INTERRUPT_MASK, 0).unwrap();
    modify(&mut bus, ADC120 + START_SRC, 0, START_CONT);

    // adc_done0_irq_handler: read the prefetched channel, prefetch ADC_SEL.
    let mut read_channel = 1u32;
    let mut seen = Vec::new();
    for _ in 0..4 {
        bus.tick_peripherals();
        let irq = bus.read_u32(ADC120 + INTERRUPT).unwrap();
        assert_ne!(irq & DONE0, 0, "averaged sample ready");
        // The comparator sees every sample: undriven channel 1 (0) is
        // below A, the 25 °C divider on channel 3 is not.
        assert_eq!(
            irq & CMPA != 0,
            read_channel == 1,
            "CMPA after channel {read_channel}"
        );
        bus.write_u32(ADC120 + INTERRUPT, 0xF).unwrap();
        seen.push((read_channel, sample(&bus, ADC120, read_channel)));
        read_channel = bus.read_u32(ADC120 + SEL).unwrap();
    }
    assert_eq!(
        seen,
        vec![(1, 0), (3, ntc_25), (1, 0), (3, ntc_25)],
        "the sequencer rotates 1 -> 3 -> 1; only P0_3 is driven"
    );

    // The NTC warms: the next channel-3 sample drops under A and CMPA fires.
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_80 = code(signal_voltage(&session, "board.analog.P0_3_volts"));
    assert!(ntc_80 < ntc_25 - 200);
    // The next conversion is channel 1 (prefetched); clear its CMPA, then
    // channel 3 converts the warm divider.
    assert_eq!(read_channel, 1);
    bus.tick_peripherals();
    bus.write_u32(ADC120 + INTERRUPT, 0xF).unwrap();
    assert_eq!(bus.read_u32(ADC120 + SEL).unwrap(), 3);
    bus.tick_peripherals();
    assert_eq!(
        sample(&bus, ADC120, 3),
        ntc_80,
        "continuous scan follows the NTC"
    );
    assert_ne!(
        bus.read_u32(ADC120 + INTERRUPT).unwrap() & CMPA,
        0,
        "below threshold A"
    );
    assert!(adc_irq(&bus, "adc120"));

    // ADC_Stop: mask, CONT off; the results freeze.
    bus.write_u32(ADC120 + INTERRUPT_MASK, 0xF).unwrap();
    assert!(!adc_irq(&bus, "adc120"), "masked");
    modify(&mut bus, ADC120 + START_SRC, START_CONT, 0);
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    bus.tick_peripherals();
    assert_eq!(
        sample(&bus, ADC120, 3),
        ntc_80,
        "stopped scans do not update"
    );
}

#[test]
fn ae822_p0_0_and_p1_4_convert_after_the_netlists_move() {
    let (mut bus, mut session) = board("P0_0", "P1_4");
    for adc in [ADC120, ADC121, ADC122] {
        zephyr_init(&mut bus, adc);
    }
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 80.0);
    let ntc_v = signal_voltage(&session, "board.analog.P0_0_volts");
    let diode_v = signal_voltage(&session, "board.analog.P1_4_volts");
    assert_eq!(
        zephyr_read(&mut bus, ADC120, "adc120", 0),
        code(ntc_v),
        "P0_0"
    );
    assert_eq!(
        zephyr_read(&mut bus, ADC122, "adc122", 0),
        code(diode_v),
        "P1_4"
    );
    assert_eq!(
        zephyr_read(&mut bus, ADC120, "adc120", 3),
        0,
        "P0_3 is not driven"
    );
    assert_eq!(
        zephyr_read(&mut bus, ADC121, "adc121", 4),
        0,
        "P1_2 is not driven"
    );
}
