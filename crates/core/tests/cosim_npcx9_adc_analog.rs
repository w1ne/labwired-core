// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The generic NTC divider and diode load line reach the NPCX9M6F EVB's ADC0
//! (`npcx_adc`) through the onboarding descriptor's `analog_pins` map
//! (Zephyr npcx9-pinctrl.dtsi pads): the NTC on GPIO45 (CH0) and the diode
//! on GPIOC7 (CH11), read the way Zephyr's `adc_npcx` scan does it (`ADCEN`,
//! `STOP`, clear `EOCCEV`, `ADCCS`, `ADCMD` = scan, `INTECCEN`, `START`,
//! wait for `EOCCEV`, read `CHNDAT[9:0]`) at 10 bits against 2816 mV. Also:
//! - `INTECCEN` pends IRQ 10 in the NVIC when the cycle ends;
//! - the two netlists moved to GPIOF1 (CH8) and GPIO34 (CH6) convert in the
//!   Chromium EC channel mode (`ASCADD`, `EOCEV`, `CHNDAT.NEW`), and the old
//!   pads go back to 0 V;
//! - one repetitive scan (`ADCRPTC`) started once follows the NTC, and a
//!   threshold detector (`THRCTL1`, `L_H` = 1: data <= THRVAL) on CH0 pends
//!   IRQ 10 only once the NTC is hot.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;
use std::sync::atomic::Ordering;

const ADC0: u64 = 0x400D_1000;
const ADCSTS: u64 = 0x00;
const ADCCNF: u64 = 0x02;
const ASCADD: u64 = 0x06;
const ADCCS: u64 = 0x08;
const THRCTS: u64 = 0x1A;
const CHNDAT0: u64 = 0x40;
const THRCTL1: u64 = 0x60;
const EOCEV: u16 = 1 << 0;
const EOCCEV: u16 = 1 << 1;
const ADCEN: u16 = 1 << 0;
const SCAN: u16 = 1 << 1;
const ADCRPTC: u16 = 1 << 3;
const START: u16 = 1 << 4;
const INTECEN: u16 = 1 << 6;
const INTECCEN: u16 = 1 << 7;
const STOP: u16 = 1 << 11;
const NEW: u16 = 1 << 15;
const THEN: u16 = 1 << 15;
const L_H: u16 = 1 << 14;
const THR1_IEN: u16 = 1 << 8;
const ADC_IRQ: u32 = 10;
/// npcx9-pinctrl.dtsi: CH0 = GPIO45, CH6 = GPIO34, CH8 = GPIOF1,
/// CH11 = GPIOC7.
const CH0: u8 = 0;
const CH6: u8 = 6;
const CH8: u8 = 8;
const CH11: u8 = 11;

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

fn millivolts(volts: f64) -> i64 {
    (volts * 1000.0).round() as i64
}

/// 10 bits, EC `ADC_MAX_VOLT` 2816 mV over 1024 codes, truncated, clamped.
fn code(volts: f64) -> u16 {
    (millivolts(volts) * 1024 / 2816).clamp(0, 1023) as u16
}

/// The onboarding NPCX9M6F EVB with the generic NTC and diode netlists routed
/// to the two pads.
fn board(ntc_pad: &str, diode_pad: &str) -> (SystemBus, CosimSession) {
    let system_path = root("configs/systems/onboarding/nuvoton_npcx9m6fb_evb.yaml");
    let mut profile = SystemManifest::from_file(&system_path).expect("load NPCX9 EVB profile");
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
    let chip = ChipDescriptor::from_file(&chip_path).expect("load NPCX9 descriptor");
    let bus = SystemBus::from_config(&chip, &profile).expect("build NPCX9 bus");
    let session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("fixture carries both analog devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{ntc_pad}/{diode_pad} must resolve to ADC0 inputs through analog_pins"
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

fn chndat(bus: &mut SystemBus, ch: u8) -> u16 {
    bus.read_u16(ADC0 + CHNDAT0 + 2 * u64::from(ch))
        .expect("CHNDAT")
}

/// Zephyr `adc_npcx_start_scan` over `channels`; returns after `EOCCEV`.
fn scan(bus: &mut SystemBus, channels: u16, extra: u16) {
    bus.write_u16(ADC0 + ADCCNF, ADCEN | STOP)
        .expect("ADCEN, STOP");
    bus.write_u16(ADC0 + ADCSTS, EOCCEV).expect("clear EOCCEV");
    bus.write_u16(ADC0 + ADCCS, channels).expect("ADCCS");
    bus.write_u16(ADC0 + ADCCNF, ADCEN | SCAN | extra)
        .expect("scan mode");
    bus.write_u16(ADC0 + ADCCNF, ADCEN | SCAN | extra | START)
        .expect("START");
    assert_ne!(
        bus.read_u16(ADC0 + ADCSTS).unwrap() & EOCCEV,
        0,
        "the scan cycle ends in EOCCEV"
    );
}

/// Chromium EC `start_single_and_wait` + `adc_read_channel`.
fn read_channel(bus: &mut SystemBus, ch: u8) -> u16 {
    bus.write_u16(ADC0 + ADCCNF, ADCEN | STOP).expect("STOP");
    bus.write_u16(ADC0 + ASCADD, u16::from(ch)).expect("ASCADD");
    bus.write_u16(ADC0 + ADCSTS, EOCEV).expect("clear EOCEV");
    bus.write_u16(ADC0 + ADCCNF, ADCEN | INTECEN | START)
        .expect("START");
    assert_ne!(bus.read_u16(ADC0 + ADCSTS).unwrap() & EOCEV, 0, "EOCEV");
    let dat = chndat(bus, ch);
    assert_ne!(dat & NEW, 0, "CHNDAT.NEW");
    assert_eq!(bus.read_u16(ADC0 + ASCADD).unwrap(), u16::from(ch));
    bus.write_u16(ADC0 + ADCSTS, EOCEV).expect("EOCEV w1c");
    dat & 0x3FF
}

fn nvic_pending(bus: &SystemBus) -> Option<bool> {
    bus.nvic
        .as_ref()
        .map(|n| n.ispr[0].load(Ordering::SeqCst) & (1 << ADC_IRQ) != 0)
}

#[test]
fn npcx9_gpio45_and_gpioc7_convert_ntc_and_diode_in_a_zephyr_scan() {
    let (mut bus, mut session) = board("gpio45", "gpioc7");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut cycles = 0;
    let mut ntc_codes = Vec::new();
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.gpio45_volts");
        assert!((ntc_v - ntc_volts(temperature_c)).abs() < 1e-9);
        let diode_v = signal_voltage(&session, "board.analog.gpioc7_volts");
        assert!(diode_v > 0.5 && diode_v < 0.8, "diode drop {diode_v} V");

        scan(&mut bus, (1 << CH0) | (1 << CH11) | (1 << CH6), INTECCEN);
        bus.tick_peripherals_fully();
        assert_eq!(nvic_pending(&bus), Some(true), "INTECCEN pends IRQ 10");
        let status = bus.read_u16(ADC0 + ADCSTS).unwrap();
        bus.write_u16(ADC0 + ADCSTS, status)
            .expect("ISR clears status");
        let ntc_code = chndat(&mut bus, CH0);
        assert_eq!(ntc_code, NEW | code(ntc_v), "CH0 at {temperature_c} °C");
        assert_eq!(chndat(&mut bus, CH11), NEW | code(diode_v), "CH11");
        assert_eq!(chndat(&mut bus, CH6), NEW, "unrouted GPIO34 reads 0");
        assert_eq!(chndat(&mut bus, CH8), 0, "CH8 not in ADCCS");
        ntc_codes.push(ntc_code & 0x3FF);
        // Clear the pending bit, as the NVIC does on exception entry.
        bus.nvic.as_ref().unwrap().ispr[0].fetch_and(!(1 << ADC_IRQ), Ordering::SeqCst);
    }
    assert!(
        ntc_codes[0] > ntc_codes[1] && ntc_codes[1] > ntc_codes[2],
        "the NTC codes fall as it warms: {ntc_codes:?}"
    );
    // The -10 °C divider (2.816 V) lands on VREF itself: full scale.
    assert_eq!(ntc_codes[0], 1023, "-10 °C saturates at 2816 mV");
    // Zephyr adc_raw_to_millivolts: raw * vref >> 10 recovers the divider.
    let mv = (i64::from(ntc_codes[1]) * 2816) >> 10;
    assert!(
        (mv - millivolts(ntc_volts(25.0))).abs() <= 3,
        "adc_raw_to_millivolts gives {mv} mV"
    );
}

#[test]
fn npcx9_gpiof1_and_gpio34_convert_in_ec_channel_mode() {
    let (mut bus, mut session) = board("gpiof1", "gpio34");
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, 25.0);
    let ntc_v = signal_voltage(&session, "board.analog.gpiof1_volts");
    let diode_v = signal_voltage(&session, "board.analog.gpio34_volts");
    assert_eq!(read_channel(&mut bus, CH8), code(ntc_v), "CH8 (GPIOF1)");
    assert_eq!(read_channel(&mut bus, CH6), code(diode_v), "CH6 (GPIO34)");
    assert_eq!(read_channel(&mut bus, CH0), 0, "GPIO45 is no longer driven");
    assert_eq!(
        read_channel(&mut bus, CH11),
        0,
        "GPIOC7 is no longer driven"
    );
}

#[test]
fn npcx9_repetitive_scan_follows_the_ntc_and_a_threshold_flags_overtemperature() {
    let (mut bus, mut session) = board("gpio45", "gpioc7");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut cycles = 0;
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    // adc_npcx_threshold: THR1 on CH0, L_H = 1 (data <= THRVAL), 1.0 V.
    let thrval = code(1.0);
    bus.write_u16(ADC0 + THRCTL1, THEN | L_H | (u16::from(CH0) << 10) | thrval)
        .expect("THRCTL1");
    bus.write_u16(ADC0 + THRCTS, THR1_IEN).expect("THR1_IEN");
    // adc_npcx_set_repetitive: scan, ADCRPTC, START once.
    scan(&mut bus, 1 << CH0, ADCRPTC);
    for temperature_c in [-10.0_f64, 25.0, 80.0] {
        settle(&mut session, &mut bus, &mut cycles, temperature_c);
        let ntc_v = signal_voltage(&session, "board.analog.gpio45_volts");
        bus.tick_peripherals_fully();
        assert_eq!(
            chndat(&mut bus, CH0) & 0x3FF,
            code(ntc_v),
            "repetitive result at {temperature_c} °C"
        );
        let hot = ntc_v <= 1.0;
        assert_eq!(hot, temperature_c > 50.0, "only 80 °C is under 1 V");
        assert_eq!(
            bus.read_u16(ADC0 + THRCTS).unwrap() & 1,
            u16::from(hot),
            "THR1 status at {temperature_c} °C"
        );
        assert_eq!(nvic_pending(&bus), Some(hot), "THR1 pends IRQ 10");
    }
    // The ISR clears THR1 alone; the NTC is still hot, so the next cycle
    // raises it again.
    let thrcts = (bus.read_u16(ADC0 + THRCTS).unwrap() & !0x3F) | 1;
    bus.write_u16(ADC0 + THRCTS, thrcts).expect("THR1 w1c");
    assert_eq!(bus.read_u16(ADC0 + THRCTS).unwrap(), THR1_IEN);
    bus.tick_peripherals_fully();
    assert_eq!(bus.read_u16(ADC0 + THRCTS).unwrap() & 1, 1);
    // STOP ends the repetitive cycle: cooling no longer updates CHNDAT.
    let hot_code = chndat(&mut bus, CH0) & 0x3FF;
    bus.write_u16(ADC0 + ADCCNF, ADCEN | SCAN | ADCRPTC | STOP)
        .expect("STOP");
    settle(&mut session, &mut bus, &mut cycles, -10.0);
    bus.tick_peripherals_fully();
    assert_eq!(chndat(&mut bus, CH0) & 0x3FF, hot_code);
}
